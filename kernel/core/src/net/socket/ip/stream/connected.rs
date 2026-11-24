use aster_bigtcp::{
    errors::tcp::{IoError, RecvError, SendError},
    socket::{NeedIfacePoll, RawTcpSetOption},
    wire::IpEndpoint,
};

use super::observer::StreamObserver;
use crate::{
    events::IoEvents,
    net::{
        iface::{BoundTcpPort, Iface, RawTcpSocketExt, TcpConnection},
        socket::util::{LingerOption, RecvFlags, SendFlags, SockShutdownCmd},
    },
    prelude::*,
    process::signal::{Pollee, Poller},
    util::{MultiRead, MultiWrite},
};

pub(super) struct ConnectedStream {
    tcp_conn: TcpConnection,
    remote_endpoint: IpEndpoint,
    /// Indicates whether this connection is "new" in a `connect()` system call.
    ///
    /// If the connection is not new, `connect()` will fail with the error code `EISCONN`,
    /// otherwise it will succeed. This means that `connect()` will succeed _exactly_ once,
    /// regardless of whether the connection is established synchronously or asynchronously.
    ///
    /// If the connection is established synchronously, the synchronous `connect()` will succeed
    /// and any subsequent `connect()` will fail; otherwise, the first `connect()` after the
    /// connection is established asynchronously will succeed and any subsequent `connect()` will
    /// fail.
    is_new_connection: bool,
}

impl ConnectedStream {
    pub(super) fn new(
        tcp_conn: TcpConnection,
        remote_endpoint: IpEndpoint,
        is_new_connection: bool,
    ) -> Self {
        Self {
            tcp_conn,
            remote_endpoint,
            is_new_connection,
        }
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd, pollee: &Pollee) -> Result<()> {
        let mut events = IoEvents::empty();

        if cmd.shut_read() {
            if !self.tcp_conn.shut_recv() {
                return Err(errno!(ENOTCONN, "the socket is not connected"));
            }
            events |= IoEvents::IN | IoEvents::RDHUP;
        }

        if cmd.shut_write() {
            if !self.tcp_conn.shut_send() {
                return Err(errno!(ENOTCONN, "the socket is not connected"));
            }
            events |= IoEvents::OUT | IoEvents::HUP;
        }

        pollee.notify(events);

        Ok(())
    }

    pub(super) fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: RecvFlags,
    ) -> Result<(usize, NeedIfacePoll)> {
        let mut trunc_len = if flags.contains(RecvFlags::MSG_TRUNC) {
            Some(writer.sum_lens())
        } else {
            None
        };
        let result = self
            .tcp_conn
            .recv(flags.receive_behavior(), |socket_buffer| {
                if let Some(remaining) = trunc_len.as_mut() {
                    let recv_len = socket_buffer.len().min(*remaining);
                    *remaining -= recv_len;
                    Ok(recv_len)
                } else {
                    writer.write(&mut VmReader::from(socket_buffer))
                }
            });

        match result {
            Ok((recv_bytes, need_poll)) => Ok((recv_bytes.get(), need_poll)),
            Err(IoError::NoProgress) => Err(errno!(EAGAIN, "the receive buffer is empty")),
            Err(IoError::Copy(e)) => Err(e.into()),
            Err(IoError::Socket(RecvError::Finished | RecvError::InvalidState)) => {
                Ok((0, NeedIfacePoll::FALSE))
            }
            Err(IoError::Socket(RecvError::ConnReset)) => {
                Err(errno!(ECONNRESET, "the connection is reset"))
            }
        }
    }

    pub(super) fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        _flags: SendFlags,
    ) -> Result<(usize, NeedIfacePoll)> {
        let result = self
            .tcp_conn
            .send(|socket_buffer| reader.read(&mut VmWriter::from(socket_buffer)));

        match result {
            Ok((sent_bytes, need_poll)) => Ok((sent_bytes.get(), need_poll)),
            Err(IoError::NoProgress) => Err(errno!(EAGAIN, "the send buffer is full")),
            Err(IoError::Copy(e)) => Err(e.into()),
            Err(IoError::Socket(SendError::InvalidState)) => {
                Err(errno!(EPIPE, "the connection is closed"))
            }
            Err(IoError::Socket(SendError::ConnReset)) => {
                Err(errno!(ECONNRESET, "the connection is reset"))
            }
        }
    }

    pub(super) fn local_endpoint(&self) -> IpEndpoint {
        self.tcp_conn.local_endpoint()
    }

    pub(super) fn remote_endpoint(&self) -> IpEndpoint {
        self.remote_endpoint
    }

    pub(super) fn iface(&self) -> &Arc<Iface> {
        self.tcp_conn.iface()
    }

    pub(super) fn bound_port(&self) -> &BoundTcpPort {
        self.tcp_conn.bound_port()
    }

    pub(super) fn finish_last_connect(&mut self) -> Result<()> {
        if !self.is_new_connection {
            return Err(errno!(EISCONN, "the socket is already connected"));
        }

        self.is_new_connection = false;
        Ok(())
    }

    pub(super) fn init_observer(&self, observer: StreamObserver) {
        self.tcp_conn.init_observer(observer);
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        self.tcp_conn.raw_with(|socket| {
            let is_receiving_closed = socket.is_recv_shut() || !socket.may_recv_new();
            let is_sending_closed = !socket.may_send();

            let mut events = IoEvents::empty();

            if is_receiving_closed {
                events |= IoEvents::IN | IoEvents::RDHUP;
            } else if socket.can_recv() {
                events |= IoEvents::IN;
            }

            if is_sending_closed || socket.can_send() {
                events |= IoEvents::OUT;
            }

            if is_receiving_closed && is_sending_closed {
                events |= IoEvents::HUP;
            }

            if socket.is_rst_closed() {
                events |= IoEvents::ERR;
            }

            events
        })
    }

    pub(super) fn test_and_clear_error(&self) -> Option<Error> {
        if self.tcp_conn.clear_rst_closed() {
            Some(errno!(ECONNRESET, "the connection is reset",))
        } else {
            None
        }
    }

    pub(super) fn set_raw_option<R>(
        &self,
        set_option: impl FnOnce(&dyn RawTcpSetOption) -> R,
    ) -> R {
        set_option(&self.tcp_conn)
    }

    pub(super) fn raw_with<R>(&self, f: impl FnOnce(&RawTcpSocketExt) -> R) -> R {
        self.tcp_conn.raw_with(f)
    }

    pub(super) fn into_connection(self) -> TcpConnection {
        self.tcp_conn
    }
}

pub(super) fn close_and_linger(tcp_conn: TcpConnection, linger: LingerOption, pollee: &Pollee) {
    let timeout = match (linger.is_on(), linger.timeout()) {
        (false, _) => {
            tcp_conn.close();
            tcp_conn.iface().poll();
            return;
        }

        (true, duration) if duration.is_zero() => {
            tcp_conn.reset();
            tcp_conn.iface().poll();
            return;
        }

        (true, duration) => {
            tcp_conn.close();
            tcp_conn.iface().poll();
            duration
        }
    };

    let mut poller = Poller::new(Some(&timeout));
    pollee.register_poller(poller.as_handle_mut(), IoEvents::HUP);

    while tcp_conn.raw_with(|socket| socket.is_closing()) {
        if poller.wait().is_err() {
            break;
        }
    }
}
