use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
};
use core::{
    num::NonZeroUsize,
    ops::{Deref, DerefMut},
};

use aster_softirq::BottomHalfDisabled;
use ostd::sync::{SpinLock, SpinLockGuard};
use smoltcp::{
    socket::{PollAt, tcp::State},
    time::Duration,
    wire::{IpEndpoint, IpRepr, TcpControl, TcpRepr},
};

use super::{
    ReceiveBehavior,
    common::{Inner, NeedIfacePoll, Socket, SocketBg},
    tcp_listen::TcpListenerBg,
};
use crate::{
    define_boolean_value,
    errors::tcp::{ConnectError, IoError, RecvError, SendError},
    ext::Ext,
    iface::{BoundTcpPort, PacketSlice, PollKey, PollableIfaceMut},
    socket::{
        event::SocketEvents,
        option::{RawTcpOption, RawTcpSetOption},
        unbound::{RawTcpSocket, new_tcp_socket},
    },
    socket_table::ConnectionKey,
};

pub type TcpConnection<E> = Socket<TcpConnectionInner<E>, E>;

pub struct TcpConnectionInner<E: Ext> {
    socket: SpinLock<RawTcpSocketExt<E>, BottomHalfDisabled>,
    poll_key: PollKey,
    connection_key: ConnectionKey,
}

pub struct RawTcpSocketExt<E: Ext> {
    socket: Box<RawTcpSocket>,
    pub(super) listener: Option<Arc<TcpListenerBg<E>>>,
    has_connected: bool,
    is_recv_shut: bool,
    is_rst_closed: bool,
}

impl<E: Ext> Deref for RawTcpSocketExt<E> {
    type Target = RawTcpSocket;

    fn deref(&self) -> &Self::Target {
        &self.socket
    }
}

impl<E: Ext> DerefMut for RawTcpSocketExt<E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.socket
    }
}

impl<E: Ext> RawTcpSocketExt<E> {
    pub fn may_recv_new(&self) -> bool {
        match self.state() {
            State::Established => true,
            State::FinWait1 | State::FinWait2 => true,
            _ => false,
        }
    }

    pub fn is_closing(&self) -> bool {
        let state = self.state();
        matches!(state, State::FinWait1 | State::Closing | State::LastAck)
    }

    pub fn is_recv_shut(&self) -> bool {
        self.is_recv_shut
    }

    pub fn is_rst_closed(&self) -> bool {
        self.is_rst_closed
    }
}

define_boolean_value!(
    pub(crate) TcpConnBecameDead
);

impl<E: Ext> RawTcpSocketExt<E> {
    fn check_state(
        &mut self,
        this: &Arc<TcpConnectionBg<E>>,
        old_state: State,
        old_recv_queue: usize,
        is_rst: bool,
    ) -> (SocketEvents, TcpConnBecameDead) {
        let became_dead = if self.state() != State::Established {
            if self.is_recv_shut
                && matches!(old_state, State::FinWait1 | State::FinWait2)
                && self.recv_queue() > old_recv_queue
            {
                self.is_rst_closed = true;
                self.abort();
            }
            self.check_dead(this)
        } else {
            TcpConnBecameDead::FALSE
        };

        let events = if self.state() != old_state {
            if self.state() == State::Closed && is_rst {
                self.is_rst_closed = true;
            }
            self.on_new_state(this)
        } else {
            SocketEvents::empty()
        };

        (events, became_dead)
    }

    fn on_new_state(&mut self, this: &Arc<TcpConnectionBg<E>>) -> SocketEvents {
        let may_send = self.may_send();

        if may_send && !self.has_connected {
            self.has_connected = true;

            if let Some(ref listener) = self.listener {
                let mut backlog = listener.inner.backlog.lock();
                if let Some(value) = backlog.connecting.remove(this.connection_key()) {
                    backlog.connected.push(value);
                }
                listener.notify_events(SocketEvents::CAN_RECV);
            }
        }

        let mut events = SocketEvents::empty();
        if !self.may_recv_new() {
            events |= SocketEvents::CLOSED_RECV;
        }
        if !may_send {
            events |= SocketEvents::CLOSED_SEND;
        }

        events
    }

    fn check_dead(&self, this: &Arc<TcpConnectionBg<E>>) -> TcpConnBecameDead {
        if self.state() == State::Closed && self.remote_endpoint().is_none() {
            return TcpConnBecameDead::TRUE;
        }

        if self.state() == State::Listen {
            if let Some(ref listener) = self.listener {
                let mut backlog = listener.inner.backlog.lock();
                let _ = backlog.connecting.remove(&this.inner.connection_key);
            }

            return TcpConnBecameDead::TRUE;
        }

        TcpConnBecameDead::FALSE
    }
}

impl<E: Ext> TcpConnectionInner<E> {
    pub(super) fn new(
        socket: Box<RawTcpSocket>,
        listener: Option<Arc<TcpListenerBg<E>>>,
        weak_self: &Weak<TcpConnectionBg<E>>,
    ) -> Self {
        let connection_key = {
            let local_endpoint = socket.local_endpoint().unwrap();
            let remote_endpoint = socket.remote_endpoint().unwrap();
            ConnectionKey::from((local_endpoint, remote_endpoint))
        };

        let poll_key = PollKey::new(Weak::as_ptr(weak_self).addr());

        let socket_ext = RawTcpSocketExt {
            socket,
            listener,
            has_connected: false,
            is_recv_shut: false,
            is_rst_closed: false,
        };

        TcpConnectionInner {
            socket: SpinLock::new(socket_ext),
            poll_key,
            connection_key,
        }
    }

    pub(super) fn lock(&self) -> SpinLockGuard<'_, RawTcpSocketExt<E>, BottomHalfDisabled> {
        self.socket.lock()
    }
}

impl<E: Ext> Inner<E> for TcpConnectionInner<E> {
    type BoundPort = BoundTcpPort<E>;
    type Observer = E::TcpEventObserver;

    fn on_drop(this: &Arc<SocketBg<Self, E>>) {
        debug_assert!(
            {
                let socket = this.inner.lock();
                if socket.state() == State::Closed {
                    true
                } else {
                    socket.is_recv_shut
                        && !matches!(
                            socket.state(),
                            State::SynSent
                                | State::SynReceived
                                | State::Established
                                | State::CloseWait,
                        )
                }
            },
            "a connection must be either closed or reset before dropping"
        );
    }
}

pub(crate) type TcpConnectionBg<E> = SocketBg<TcpConnectionInner<E>, E>;

pub enum ConnectState {
    Connecting,
    Connected,
    Refused,
}

impl<E: Ext> TcpConnection<E> {
    pub fn new_connect(
        mut bound: BoundTcpPort<E>,
        remote_endpoint: IpEndpoint,
        option: &RawTcpOption,
        observer: E::TcpEventObserver,
    ) -> Result<Self, (BoundTcpPort<E>, ConnectError)> {
        let iface = bound.iface().clone();

        let mut interface = iface.common().interface();

        let connection_key = {
            bound.ensure_unicast(&interface);
            let local_endpoint = bound.endpoint();
            ConnectionKey::from((local_endpoint, remote_endpoint))
        };

        let mut sockets = iface.common().sockets();

        if sockets.lookup_connection(&connection_key).is_some() {
            return Err((bound, ConnectError::AddressInUse));
        }

        let socket = {
            let mut socket = new_tcp_socket();

            option.apply(&mut socket);

            if let Err(err) = socket.connect(interface.context_mut(), remote_endpoint, bound.port())
            {
                return Err((bound, err.into()));
            }

            socket
        };

        let connection =
            Self::new_cyclic(bound, |weak| TcpConnectionInner::new(socket, None, weak));
        interface.update_next_poll_at_ms(&connection.0, PollAt::Now);
        connection.init_observer(observer);

        let res = sockets.insert_connection(connection.inner().clone());
        debug_assert!(res.is_ok());

        Ok(connection)
    }

    pub fn connect_state(&self) -> ConnectState {
        let socket = self.0.inner.lock();

        if socket.state() == State::SynSent || socket.state() == State::SynReceived {
            ConnectState::Connecting
        } else if socket.has_connected {
            ConnectState::Connected
        } else if Arc::strong_count(self.0.as_ref()) > 1 {
            ConnectState::Connecting
        } else {
            ConnectState::Refused
        }
    }

    pub fn into_bound_port(mut self) -> Option<BoundTcpPort<E>> {
        let this: TcpConnectionBg<E> = Arc::into_inner(self.0.take())?;
        Some(this.bound)
    }

    pub fn send<F, CopyErr>(
        &self,
        mut f: F,
    ) -> Result<(NonZeroUsize, NeedIfacePoll), IoError<SendError, CopyErr>>
    where
        F: FnMut(&mut [u8]) -> Result<usize, (CopyErr, usize)>,
    {
        let common = self.iface().common();
        let mut iface = common.interface();

        let mut socket = self.0.inner.lock();

        if socket.is_rst_closed {
            socket.is_rst_closed = false;
            return Err(IoError::Socket(SendError::ConnReset));
        }

        let mut total_sent_bytes = 0;
        let mut need_wrap_continuation = true;

        let result = loop {
            let mut sent_bytes = 0;
            let mut socket_buffer_len = 0;

            let send_result = socket.send(|socket_buffer| {
                socket_buffer_len = socket_buffer.len();
                match f(socket_buffer) {
                    Ok(current_sent_bytes) => {
                        sent_bytes = current_sent_bytes;
                        (current_sent_bytes, Ok(()))
                    }
                    Err((err, current_sent_bytes)) => {
                        sent_bytes = current_sent_bytes;
                        (current_sent_bytes, Err(err))
                    }
                }
            });

            let copy_result = match send_result {
                Err(_) if total_sent_bytes > 0 => break total_sent_bytes,
                res => res?,
            };

            total_sent_bytes += sent_bytes;

            if let Err(err) = copy_result {
                if total_sent_bytes == 0 {
                    return Err(IoError::Copy(err));
                }
                break total_sent_bytes;
            }

            if sent_bytes < socket_buffer_len || !need_wrap_continuation {
                break total_sent_bytes;
            }

            need_wrap_continuation = false;
        };

        let result = NonZeroUsize::new(result).ok_or(IoError::NoProgress)?;
        let poll_at = socket.poll_at(iface.context_mut());
        let need_poll = iface.update_next_poll_at_ms(&self.0, poll_at);

        Ok((result, need_poll))
    }

    pub fn recv<CopyFn, CopyErr>(
        &self,
        behavior: ReceiveBehavior,
        mut copy_fn: CopyFn,
    ) -> Result<(NonZeroUsize, NeedIfacePoll), IoError<RecvError, CopyErr>>
    where
        CopyFn: FnMut(&[u8]) -> Result<usize, (CopyErr, usize)>,
    {
        let common = self.iface().common();
        let mut iface = common.interface();

        let mut socket = self.0.inner.lock();

        if socket.is_recv_shut && socket.recv_queue() == 0 {
            return Err(IoError::Socket(RecvError::Finished));
        }

        let mut total_recv_bytes = 0;
        let mut need_wrap_continuation = behavior.will_consume_data();

        let result = loop {
            let mut recv_bytes = 0;
            let mut socket_buffer_len = 0;

            let mut copy_socket_buffer = |socket_buffer: &[u8]| {
                socket_buffer_len = socket_buffer.len();
                match copy_fn(socket_buffer) {
                    Ok(current_recv_bytes) => {
                        recv_bytes = current_recv_bytes;
                        (current_recv_bytes, Ok(()))
                    }
                    Err((err, current_recv_bytes)) => {
                        recv_bytes = current_recv_bytes;
                        (current_recv_bytes, Err(err))
                    }
                }
            };

            let recv_result = match behavior {
                ReceiveBehavior::Recv => {
                    socket.recv(|socket_buffer| copy_socket_buffer(socket_buffer))
                }
                ReceiveBehavior::Peek => {
                    let recv_queue = socket.recv_queue();
                    socket.peek(recv_queue).map(|socket_buffer| {
                        let (_, copy_result) = copy_socket_buffer(socket_buffer);
                        copy_result
                    })
                }
            };

            let copy_result = match recv_result {
                Err(_) if socket.is_rst_closed && total_recv_bytes == 0 => {
                    socket.is_rst_closed = false;
                    return Err(IoError::Socket(RecvError::ConnReset));
                }
                Err(_) if total_recv_bytes > 0 => break total_recv_bytes,
                res => res?,
            };

            total_recv_bytes += recv_bytes;

            if let Err(err) = copy_result {
                if total_recv_bytes == 0 {
                    return Err(IoError::Copy(err));
                }
                break total_recv_bytes;
            }

            if recv_bytes < socket_buffer_len || !need_wrap_continuation {
                break total_recv_bytes;
            }

            need_wrap_continuation = false;
        };

        let result = NonZeroUsize::new(result).ok_or(IoError::NoProgress)?;
        let poll_at = socket.poll_at(iface.context_mut());
        let need_poll = iface.update_next_poll_at_ms(&self.0, poll_at);

        Ok((result, need_poll))
    }

    pub fn clear_rst_closed(&self) -> bool {
        let mut socket = self.0.inner.lock();

        let is_rst = socket.is_rst_closed;
        socket.is_rst_closed = false;
        is_rst
    }

    pub fn shut_send(&self) -> bool {
        let mut iface = self.iface().common().interface();
        let mut socket = self.0.inner.lock();

        if matches!(socket.state(), State::Closed | State::TimeWait) {
            return false;
        }

        socket.close();

        let poll_at = socket.poll_at(iface.context_mut());
        iface.update_next_poll_at_ms(&self.0, poll_at);

        true
    }

    pub fn shut_recv(&self) -> bool {
        let mut socket = self.0.inner.lock();

        if matches!(socket.state(), State::Closed | State::TimeWait) {
            return false;
        }

        socket.is_recv_shut = true;

        true
    }

    pub fn close(&self) {
        let mut iface = self.iface().common().interface();
        let mut socket = self.0.inner.lock();

        socket.is_recv_shut = true;

        if socket.recv_queue() != 0 {
            socket.abort();
        } else {
            socket.close();
        }

        let poll_at = socket.poll_at(iface.context_mut());
        iface.update_next_poll_at_ms(&self.0, poll_at);
    }

    pub fn reset(&self) {
        let mut iface = self.iface().common().interface();
        let mut socket = self.0.inner.lock();

        socket.abort();

        let poll_at = socket.poll_at(iface.context_mut());
        iface.update_next_poll_at_ms(&self.0, poll_at);
    }

    pub fn raw_with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&RawTcpSocketExt<E>) -> R,
    {
        let socket = self.0.inner.lock();
        f(&socket)
    }
}

impl<E: Ext> RawTcpSetOption for TcpConnection<E> {
    fn set_keep_alive(&self, interval: Option<Duration>) -> NeedIfacePoll {
        let mut iface = self.iface().common().interface();
        let mut socket = self.0.inner.lock();

        socket.set_keep_alive(interval);

        let poll_at = socket.poll_at(iface.context_mut());
        iface.update_next_poll_at_ms(&self.0, poll_at)
    }

    fn set_nagle_enabled(&self, enabled: bool) {
        let mut socket = self.0.inner.lock();
        socket.set_nagle_enabled(enabled);
    }
}

impl<E: Ext> TcpConnectionBg<E> {
    pub(crate) const fn poll_key(&self) -> &PollKey {
        &self.inner.poll_key
    }

    pub(crate) const fn connection_key(&self) -> &ConnectionKey {
        &self.inner.connection_key
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TcpProcessResult {
    NotProcessed,
    Processed,
    ProcessedWithReply(IpRepr, TcpRepr<'static>),
}

impl<E: Ext> TcpConnectionBg<E> {
    pub(crate) fn process(
        self: &Arc<Self>,
        iface: &mut PollableIfaceMut<E>,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
        payload: PacketSlice<'_>,
    ) -> (TcpProcessResult, TcpConnBecameDead) {
        let mut socket = self.inner.lock();

        if !socket.accepts(iface.context_mut(), ip_repr, tcp_repr) {
            return (TcpProcessResult::NotProcessed, TcpConnBecameDead::FALSE);
        }

        if socket.state() == State::TimeWait
            && tcp_repr.control == TcpControl::Syn
            && tcp_repr.ack_number.is_none()
        {
            socket.abort();
            socket
                .dispatch(iface.context_mut(), |_, _| {
                    Ok::<(), core::convert::Infallible>(())
                })
                .unwrap();

            iface.update_next_poll_at_ms(self, PollAt::Ingress);
            return (TcpProcessResult::NotProcessed, TcpConnBecameDead::TRUE);
        }

        let old_state = socket.state();
        let old_recv_queue = socket.recv_queue();
        let is_rst = tcp_repr.control == TcpControl::Rst;
        let mut events = SocketEvents::CAN_RECV | SocketEvents::CAN_SEND;

        let result = match socket.process(iface.context_mut(), ip_repr, tcp_repr, payload) {
            None => TcpProcessResult::Processed,
            Some((ip_repr, tcp_repr)) => TcpProcessResult::ProcessedWithReply(ip_repr, tcp_repr),
        };

        let (state_events, became_dead) =
            socket.check_state(self, old_state, old_recv_queue, is_rst);
        events |= state_events;

        self.notify_events(events);

        let poll_at = socket.poll_at(iface.context_mut());
        iface.update_next_poll_at_ms(self, poll_at);

        (result, became_dead)
    }

    pub(crate) fn dispatch<D>(
        self: &Arc<Self>,
        iface: &mut PollableIfaceMut<E>,
        dispatch: D,
    ) -> (Option<(IpRepr, TcpRepr<'static>)>, TcpConnBecameDead)
    where
        D: FnOnce(PollableIfaceMut<E>, &IpRepr, &TcpRepr) -> Option<(IpRepr, TcpRepr<'static>)>,
    {
        let mut socket = self.inner.lock();

        let old_state = socket.state();
        let old_recv_queue = socket.recv_queue();
        let mut is_rst = false;
        let mut events = SocketEvents::empty();

        let mut reply = None;
        let (cx, pending) = iface.inner_mut();
        socket
            .dispatch(cx, |cx, (ip_repr, tcp_repr)| {
                reply = dispatch(PollableIfaceMut::new(cx, pending), &ip_repr, &tcp_repr);
                Ok::<(), ()>(())
            })
            .unwrap();

        while let Some((ref ip_repr, ref tcp_repr)) = reply {
            if !socket.accepts(iface.context_mut(), ip_repr, tcp_repr) {
                break;
            }
            is_rst |= tcp_repr.control == TcpControl::Rst;
            events |= SocketEvents::CAN_RECV | SocketEvents::CAN_SEND;
            reply = socket.process(
                iface.context_mut(),
                ip_repr,
                tcp_repr,
                PacketSlice::from(tcp_repr.payload),
            );
        }

        let (state_events, became_dead) =
            socket.check_state(self, old_state, old_recv_queue, is_rst);
        events |= state_events;

        self.notify_events(events);

        let poll_at = socket.poll_at(iface.context_mut());
        iface.update_next_poll_at_ms(self, poll_at);

        (reply, became_dead)
    }
}
