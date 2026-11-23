use smoltcp::time::Duration;

use super::{NeedIfacePoll, unbound::RawTcpSocket};

pub trait RawTcpSetOption {
    fn set_keep_alive(&self, interval: Option<Duration>) -> NeedIfacePoll;

    fn set_nagle_enabled(&self, enabled: bool);
}

pub struct RawTcpOption {
    pub keep_alive: Option<Duration>,
    pub is_nagle_enabled: bool,
}

impl RawTcpOption {
    pub(super) fn apply(&self, socket: &mut RawTcpSocket) {
        socket.set_keep_alive(self.keep_alive);
        socket.set_nagle_enabled(self.is_nagle_enabled);
    }

    pub(super) fn inherit(from: &RawTcpSocket, to: &mut RawTcpSocket) {
        to.set_keep_alive(from.keep_alive());
        to.set_nagle_enabled(from.nagle_enabled());
    }
}
