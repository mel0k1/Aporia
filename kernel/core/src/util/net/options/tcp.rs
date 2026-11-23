use super::{RawSocketOption, SocketOption, impl_raw_socket_option};
use crate::{
    net::socket::ip::stream_options::{
        Congestion, DeferAccept, Inq, KeepCnt, KeepIdle, KeepIntvl, MaxSegment, NoDelay, SynCnt,
        UserTimeout, WindowClamp,
    },
    prelude::*,
};

#[expect(non_camel_case_types)]
#[expect(clippy::upper_case_acronyms)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub(crate) enum CTcpOptionName {
    NODELAY = 1,
    MAXSEG = 2,
    CORK = 3,
    KEEPIDLE = 4,
    KEEPINTVL = 5,
    KEEPCNT = 6,
    SYNCNT = 7,
    DEFER_ACCEPT = 9,
    WINDOW_CLAMP = 10,
    CONGESTION = 13,
    USER_TIMEOUT = 18,
    INQ = 36,
}

pub(crate) fn new_tcp_option(name: i32) -> Result<Box<dyn RawSocketOption>> {
    let name = CTcpOptionName::try_from(name).map_err(|_| Errno::ENOPROTOOPT)?;
    match name {
        CTcpOptionName::NODELAY => Ok(Box::new(NoDelay::new())),
        CTcpOptionName::MAXSEG => Ok(Box::new(MaxSegment::new())),
        CTcpOptionName::KEEPIDLE => Ok(Box::new(KeepIdle::new())),
        CTcpOptionName::KEEPINTVL => Ok(Box::new(KeepIntvl::new())),
        CTcpOptionName::KEEPCNT => Ok(Box::new(KeepCnt::new())),
        CTcpOptionName::SYNCNT => Ok(Box::new(SynCnt::new())),
        CTcpOptionName::DEFER_ACCEPT => Ok(Box::new(DeferAccept::new())),
        CTcpOptionName::WINDOW_CLAMP => Ok(Box::new(WindowClamp::new())),
        CTcpOptionName::CONGESTION => Ok(Box::new(Congestion::new())),
        CTcpOptionName::USER_TIMEOUT => Ok(Box::new(UserTimeout::new())),
        CTcpOptionName::INQ => Ok(Box::new(Inq::new())),
        _ => Err(errno!(ENOPROTOOPT, "unsupported tcp-level option")),
    }
}

impl_raw_socket_option!(NoDelay);
impl_raw_socket_option!(MaxSegment);
impl_raw_socket_option!(KeepIdle);
impl_raw_socket_option!(KeepIntvl);
impl_raw_socket_option!(KeepCnt);
impl_raw_socket_option!(SynCnt);
impl_raw_socket_option!(DeferAccept);
impl_raw_socket_option!(WindowClamp);
impl_raw_socket_option!(Congestion);
impl_raw_socket_option!(UserTimeout);
impl_raw_socket_option!(Inq);
