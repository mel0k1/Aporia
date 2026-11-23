mod common;
mod tcp_conn;
mod tcp_listen;
mod udp;

pub use common::NeedIfacePoll;
pub use tcp_conn::{ConnectState, RawTcpSocketExt, TcpConnection};
pub(crate) use tcp_conn::{TcpConnectionBg, TcpProcessResult};
pub use tcp_listen::TcpListener;
pub(crate) use tcp_listen::TcpListenerBg;
pub use udp::UdpSocket;
pub(crate) use udp::UdpSocketBg;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiveBehavior {
    Recv,
    Peek,
}

impl ReceiveBehavior {
    pub fn will_consume_data(self) -> bool {
        self == Self::Recv
    }
}
