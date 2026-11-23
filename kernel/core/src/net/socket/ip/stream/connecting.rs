use aster_bigtcp::{
    errors::tcp::ConnectError,
    socket::{ConnectState, RawTcpOption, RawTcpSetOption},
    wire::IpEndpoint,
};

use super::{connected::ConnectedStream, init::InitStream, observer::StreamObserver};
use crate::{
    events::IoEvents,
    net::iface::{BoundTcpPort, Iface, TcpConnection},
    prelude::*,
};

pub(super) struct ConnectingStream {
    tcp_conn: TcpConnection,
    remote_endpoint: IpEndpoint,
}

pub(super) enum ConnResult {
    Connecting(ConnectingStream),
    Connected(ConnectedStream),
    Refused(InitStream),
}

impl ConnectingStream {
    pub(super) fn new(
        bound_port: BoundTcpPort,
        remote_endpoint: IpEndpoint,
        option: &RawTcpOption,
        observer: StreamObserver,
    ) -> Result<Self, (Error, BoundTcpPort)> {
        let tcp_conn =
            match TcpConnection::new_connect(bound_port, remote_endpoint, option, observer) {
                Ok(tcp_conn) => tcp_conn,
                Err((bound_port, ConnectError::AddressInUse)) => {
                    return Err((
                        errno!(EADDRNOTAVAIL, "connection key conflicts"),
                        bound_port,
                    ));
                }
                Err((bound_port, _)) => {
                    return Err((
                        errno!(
                            ECONNREFUSED,
                            "connecting to an unspecified address is not supported",
                        ),
                        bound_port,
                    ));
                }
            };

        Ok(Self {
            tcp_conn,
            remote_endpoint,
        })
    }

    pub(super) fn has_result(&self) -> bool {
        match self.tcp_conn.connect_state() {
            ConnectState::Connecting => false,
            ConnectState::Connected => true,
            ConnectState::Refused => true,
        }
    }

    pub(super) fn into_result(self) -> ConnResult {
        let next_state = self.tcp_conn.connect_state();

        match next_state {
            ConnectState::Connecting => ConnResult::Connecting(self),
            ConnectState::Connected => ConnResult::Connected(ConnectedStream::new(
                self.tcp_conn,
                self.remote_endpoint,
                true,
            )),
            ConnectState::Refused => {
                let bound_port = self.tcp_conn.into_bound_port().unwrap();
                ConnResult::Refused(InitStream::new_refused(bound_port))
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

    pub(super) fn check_io_events(&self) -> IoEvents {
        IoEvents::empty()
    }

    pub(super) fn set_raw_option<R>(
        &self,
        set_option: impl FnOnce(&dyn RawTcpSetOption) -> R,
    ) -> R {
        set_option(&self.tcp_conn)
    }

    pub(super) fn into_connection(self) -> TcpConnection {
        self.tcp_conn
    }
}
