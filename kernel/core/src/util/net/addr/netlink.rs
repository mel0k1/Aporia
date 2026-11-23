use super::CSocketAddrFamily;
use crate::{
    net::socket::netlink::{GroupIdSet, NetlinkSocketAddr},
    prelude::*,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct CSocketAddrNetlink {
    nl_family: u16,
    nl_pad: u16,
    nl_pid: u32,
    nl_groups: u32,
}

impl From<NetlinkSocketAddr> for CSocketAddrNetlink {
    fn from(value: NetlinkSocketAddr) -> Self {
        Self {
            nl_family: CSocketAddrFamily::AF_NETLINK as _,
            nl_pad: 0,
            nl_pid: value.port(),
            nl_groups: value.groups().as_u32(),
        }
    }
}

impl From<CSocketAddrNetlink> for NetlinkSocketAddr {
    fn from(value: CSocketAddrNetlink) -> Self {
        debug_assert_eq!(value.nl_family, CSocketAddrFamily::AF_NETLINK as u16);
        let port = value.nl_pid;
        let groups = GroupIdSet::new(value.nl_groups);
        NetlinkSocketAddr::new(port, groups)
    }
}
