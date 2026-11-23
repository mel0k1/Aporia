#![short_vis_path::add(vsock)]

use crate::{
    net::socket::vsock::transport::{BoundPort, Connection, connection::Phase},
    prelude::*,
};

pub(in vsock) enum ConnectResult {
    Connecting(Connection),
    Connected(Connection),
    Failed(BoundPort, Error),
}

impl Connection {
    pub(in vsock) fn has_connect_result(&self) -> bool {
        let state = self.inner.state.lock();
        match state.phase {
            Phase::ConnectFailed => Arc::strong_count(&self.inner) == 1,
            Phase::Connecting => false,
            Phase::Connected | Phase::Closing | Phase::Closed => true,
        }
    }

    pub(in vsock) fn finish_connect(mut self) -> ConnectResult {
        let mut state = self.inner.state.lock();
        match state.phase {
            Phase::ConnectFailed if Arc::strong_count(&self.inner) == 1 => {
                let error = state.error.take();
                drop(state);
                ConnectResult::Failed(
                    Arc::into_inner(self.inner.take()).unwrap().bound_port,
                    error.unwrap(),
                )
            }
            Phase::ConnectFailed | Phase::Connecting => {
                drop(state);
                ConnectResult::Connecting(self)
            }
            Phase::Connected | Phase::Closing | Phase::Closed => {
                drop(state);
                ConnectResult::Connected(self)
            }
        }
    }
}
