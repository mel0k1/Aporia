#![short_vis_path::add(vsock)]

mod basic;
pub(super) mod connect;
mod recv;
mod send;
mod shutdown;
mod utils;

use core::sync::atomic::AtomicUsize;

use aster_softirq::BottomHalfDisabled;
use aster_virtio::device::socket::{
    header::{VirtioVsockHdr, VirtioVsockOp, VirtioVsockShutdownFlags},
    packet::RxPacket,
};
use takeable::Takeable;

use crate::{
    events::IoEvents,
    net::socket::vsock::transport::{
        BoundPort, DEFAULT_CONNECT_TIMEOUT, DEFAULT_RX_BUF_SIZE, conn_id::ConnId,
    },
    prelude::*,
    process::signal::Pollee,
    time::Timer,
};

pub(in vsock) struct Connection {
    inner: Takeable<Arc<ConnectionInner>>,
}

impl Connection {
    pub(super) fn new(inner: Arc<ConnectionInner>) -> Self {
        Self {
            inner: Takeable::new(inner),
        }
    }
}

pub(super) struct ConnectionInner {
    conn_id: ConnId,
    bound_port: BoundPort,
    pollee: Pollee,
    state: SpinLock<ConnectionState, BottomHalfDisabled>,
    pending_tx_bytes: AtomicUsize,
}

struct ConnectionState {
    phase: Phase,
    error: Option<Error>,
    rx_queue: RxQueue,
    credit: CreditState,
    shutdown: ShutdownState,
    timer: Option<TimerState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Connecting,

    ConnectFailed,

    Connected,

    Closing,

    Closed,
}

struct RxQueue {
    packets: VecDeque<RxPacket>,
    used_bytes: usize,
    read_offset: usize,
}

struct CreditState {
    peer_buf_alloc: u32,
    peer_fwd_cnt: u32,
    local_fwd_cnt: u32,
    last_reported_fwd_cnt: u32,
    credit_request_pending: bool,
    tx_cnt: u32,
}

struct ShutdownState {
    local_read_closed: bool,
    local_write_closed: bool,
    peer_read_closed: bool,
    peer_write_closed: bool,
}

struct TimerState {
    generation: u64,
    #[expect(dead_code)]
    timer: Arc<Timer>,
}

impl ConnectionInner {
    pub(super) fn new_connecting(
        bound_port: BoundPort,
        conn_id: &ConnId,
        pollee: Pollee,
    ) -> Arc<Self> {
        pollee.invalidate();

        let this = Self::new(bound_port, conn_id, pollee, Phase::Connecting);

        let mut state = this.state.lock();
        let _ = state.send_packet(&this, VirtioVsockOp::Request, 0);
        state.arm_timeout(&this, DEFAULT_CONNECT_TIMEOUT);
        drop(state);

        this
    }

    pub(super) fn new_connected(
        bound_port: BoundPort,
        conn_id: &ConnId,
        header: &VirtioVsockHdr,
    ) -> Arc<Self> {
        let this = Self::new(bound_port, conn_id, Pollee::new(), Phase::Connected);

        let mut state = this.state.lock();
        state.update_peer_credit(&this, header);
        let _ = state.send_packet(&this, VirtioVsockOp::Response, 0);
        drop(state);

        this
    }

    fn new(bound_port: BoundPort, conn_id: &ConnId, pollee: Pollee, phase: Phase) -> Arc<Self> {
        debug_assert_eq!(bound_port.port(), conn_id.local_port);

        let peer_fully_closed = phase != Phase::Connected;

        let state = ConnectionState {
            phase,
            error: None,
            rx_queue: RxQueue {
                packets: VecDeque::new(),
                used_bytes: 0,
                read_offset: 0,
            },
            credit: CreditState {
                peer_buf_alloc: 0,
                peer_fwd_cnt: 0,
                local_fwd_cnt: 0,
                last_reported_fwd_cnt: 0,
                credit_request_pending: false,
                tx_cnt: 0,
            },
            shutdown: ShutdownState {
                local_read_closed: false,
                local_write_closed: false,
                peer_read_closed: peer_fully_closed,
                peer_write_closed: peer_fully_closed,
            },
            timer: None,
        };

        Arc::new(Self {
            conn_id: *conn_id,
            bound_port,
            pollee,
            state: SpinLock::new(state),
            pending_tx_bytes: AtomicUsize::new(0),
        })
    }

    pub(super) const fn conn_id(&self) -> ConnId {
        self.conn_id
    }

    pub(super) fn pollee(&self) -> &Pollee {
        &self.pollee
    }

    pub(super) fn on_response(&self, header: &VirtioVsockHdr) -> Result<()> {
        let mut state = self.state.lock();

        if state.phase != Phase::Connecting {
            state.active_rst(self);
            return Err(errno!(EISCONN, "the connection is established"));
        }

        state.update_peer_credit(self, header);

        state.phase = Phase::Connected;
        state.shutdown.peer_read_closed = false;
        state.shutdown.peer_write_closed = false;
        state.timer = None;

        drop(state);
        self.pollee.notify(IoEvents::OUT);

        Ok(())
    }

    pub(super) fn on_rst(&self) {
        let mut state = self.state.lock();

        state.do_rst(false);
    }

    pub(super) fn on_shutdown(&self, header: &VirtioVsockHdr) -> bool {
        let mut state = self.state.lock();
        let mut notify_events = IoEvents::empty();

        if state.phase == Phase::Connecting {
            state.active_rst(self);
            return true;
        }

        let flags = VirtioVsockShutdownFlags::from_bits_truncate(header.flags);
        if flags.contains(VirtioVsockShutdownFlags::SEND) && !state.shutdown.peer_write_closed {
            state.shutdown.peer_write_closed = true;
            notify_events |= IoEvents::IN | IoEvents::OUT | IoEvents::RDHUP | IoEvents::HUP;
        }
        if flags.contains(VirtioVsockShutdownFlags::RECEIVE) && !state.shutdown.peer_read_closed {
            state.shutdown.peer_read_closed = true;
            notify_events |= IoEvents::OUT;
        }

        if notify_events.is_empty() {
            return false;
        }

        let peer_fully_closed = state.shutdown.peer_read_closed && state.shutdown.peer_write_closed;
        let should_remove = if peer_fully_closed {
            state.phase = Phase::Closed;
            let _ = state.send_packet(self, VirtioVsockOp::Rst, 0);
            true
        } else {
            false
        };

        drop(state);
        self.pollee.notify(notify_events);

        should_remove
    }

    pub(super) fn on_rw(&self, header: &VirtioVsockHdr, packet: RxPacket) -> Result<()> {
        let mut state = self.state.lock();

        if state.shutdown.peer_write_closed {
            state.active_rst(self);
            return Err(errno!(ENOTCONN, "the connection is not established"));
        }

        let len = packet.payload_len();
        if state.rx_queue.used_bytes + len > DEFAULT_RX_BUF_SIZE {
            state.active_rst(self);
            return Err(errno!(ENOMEM, "the receive queue is full"));
        }

        state.update_peer_credit(self, header);

        if len != 0 {
            state.rx_queue.used_bytes += len;
            state.rx_queue.packets.push_back(packet);
        }

        drop(state);
        self.pollee.notify(IoEvents::IN);

        Ok(())
    }

    pub(super) fn on_credit_update(&self, header: &VirtioVsockHdr) -> Result<()> {
        let mut state = self.state.lock();

        if state.phase == Phase::Connecting {
            state.active_rst(self);
            return Err(errno!(ENOTCONN, "the connection is not established"));
        }

        state.update_peer_credit(self, header);

        state.credit.credit_request_pending = false;

        Ok(())
    }

    pub(super) fn on_credit_request(&self, header: &VirtioVsockHdr) -> Result<()> {
        let mut state = self.state.lock();

        if state.phase == Phase::Connecting {
            state.active_rst(self);
            return Err(errno!(ENOTCONN, "the connection is not established"));
        }

        state.update_peer_credit(self, header);

        let _ = state.send_packet(self, VirtioVsockOp::CreditUpdate, 0);

        Ok(())
    }

    pub(super) fn on_timeout(&self, generation: u64) -> bool {
        let mut state = self.state.lock();

        let Some(timer) = state.timer.as_ref() else {
            return false;
        };
        if timer.generation != generation {
            return false;
        }

        state.active_rst(self);

        if state.phase == Phase::ConnectFailed {
            state.error = Some(errno!(ETIMEDOUT, "the connection timed out",));
        }

        true
    }

    pub(super) fn active_rst(&self) {
        let mut state = self.state.lock();

        state.active_rst(self);
    }
}

impl ConnectionState {
    fn active_rst(&mut self, conn: &ConnectionInner) {
        if self.do_rst(true) {
            let _ = self.send_packet(conn, VirtioVsockOp::Rst, 0);
        }
    }

    fn do_rst(&mut self, is_active: bool) -> bool {
        match self.phase {
            Phase::Connecting => {
                self.phase = Phase::ConnectFailed;
                self.error = Some(errno!(ECONNRESET, "the connection is refused",));
                self.timer = None;

                true
            }
            Phase::Connected => {
                self.phase = Phase::Closed;
                self.error = is_active.then_some(errno!(ECONNRESET, "the connection is reset",));
                self.shutdown.peer_read_closed = true;
                self.shutdown.peer_write_closed = true;

                true
            }
            Phase::Closing => {
                self.phase = Phase::Closed;
                self.shutdown.peer_read_closed = true;
                self.shutdown.peer_write_closed = true;
                self.timer = None;

                true
            }

            Phase::ConnectFailed | Phase::Closed => false,
        }
    }

    fn update_peer_credit(&mut self, conn: &ConnectionInner, header: &VirtioVsockHdr) {
        let mut should_notify = false;

        should_notify |= self.credit.peer_buf_alloc != header.buf_alloc;
        self.credit.peer_buf_alloc = header.buf_alloc;

        should_notify |= self.credit.peer_fwd_cnt != header.fwd_cnt;
        self.credit.peer_fwd_cnt = header.fwd_cnt;

        if should_notify {
            conn.pollee.notify(IoEvents::OUT);
        }
    }
}
