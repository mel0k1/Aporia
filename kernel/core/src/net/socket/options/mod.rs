use macros::impl_socket_options;

use super::util::{LingerOption, SocketTimeout};
use crate::{net::socket::unix::CUserCred, prelude::*, process::Gid, util::net::SockType};

pub(in crate::net) mod macros;

pub(crate) trait SocketOption: Any + Send + Sync + Debug {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl_socket_options!(
    pub(crate) struct ReuseAddr(bool);
    pub(crate) struct SocketType(SockType);
    pub(crate) struct Error(Option<crate::error::Error>);
    pub(crate) struct Broadcast(bool);
    pub(crate) struct SendBuf(u32);
    pub(crate) struct RecvBuf(u32);
    pub(crate) struct KeepAlive(bool);
    pub(crate) struct Priority(i32);
    pub(crate) struct Linger(LingerOption);
    pub(crate) struct RecvTimeout(SocketTimeout);
    pub(crate) struct SendTimeout(SocketTimeout);
    pub(crate) struct ReusePort(bool);
    pub(crate) struct PassCred(bool);
    pub(crate) struct PeerCred(CUserCred);
    pub(crate) struct AcceptConn(bool);
    pub(crate) struct SendBufForce(u32);
    pub(crate) struct RecvBufForce(u32);
    pub(crate) struct PeerGroups(Arc<[Gid]>);
);
