#![short_vis_path::add(vsock)]

use aster_virtio::device::socket::header::{VirtioVsockOp, VirtioVsockShutdownFlags};

use crate::{
    events::IoEvents,
    net::socket::{
        util::SockShutdownCmd,
        vsock::transport::{
            Connection, DEFAULT_CLOSE_TIMEOUT,
            connection::{ConnectionInner, ConnectionState, Phase},
        },
    },
    prelude::*,
};

impl Connection {
    pub(in vsock) fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        let notify_events = {
            let mut state = self.inner.state.lock();
            debug_assert_ne!(state.phase, Phase::Connecting);
            state.shutdown(&self.inner, cmd)
        };

        self.inner.pollee.notify(notify_events);

        Ok(())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.inner.is_usable() {
            return;
        }

        let mut state = self.inner.state.lock();

        match state.phase {
            Phase::Connecting => {
                state.active_rst(&self.inner);
                drop(state);

                let vsock_space = self.inner.bound_port.vsock_space();
                vsock_space.remove_connection(&self.inner);
            }
            Phase::ConnectFailed => {}
            Phase::Connected | Phase::Closing => {
                debug_assert_eq!(state.phase, Phase::Connected);

                let _ = state.shutdown(&self.inner, SockShutdownCmd::SHUT_RDWR);

                state.phase = Phase::Closing;
                state.arm_timeout(&self.inner, DEFAULT_CLOSE_TIMEOUT);
            }
            Phase::Closed => {}
        }
    }
}

impl ConnectionState {
    #[must_use]
    fn shutdown(&mut self, conn: &ConnectionInner, cmd: SockShutdownCmd) -> IoEvents {
        let mut notify_events = IoEvents::empty();
        let mut shutdown_flags = VirtioVsockShutdownFlags::empty();

        if cmd.shut_read() && !self.shutdown.local_read_closed {
            self.shutdown.local_read_closed = true;
            shutdown_flags |= VirtioVsockShutdownFlags::RECEIVE;
            notify_events |= IoEvents::IN | IoEvents::RDHUP | IoEvents::HUP;
        }

        if cmd.shut_write() && !self.shutdown.local_write_closed {
            self.shutdown.local_write_closed = true;
            shutdown_flags |= VirtioVsockShutdownFlags::SEND;
            notify_events |= IoEvents::HUP;
        }

        let peer_fully_closed = self.shutdown.peer_read_closed && self.shutdown.peer_write_closed;
        if !peer_fully_closed && !shutdown_flags.is_empty() {
            let _ = self.send_packet(conn, VirtioVsockOp::Shutdown, shutdown_flags.bits());
        }

        notify_events
    }
}
