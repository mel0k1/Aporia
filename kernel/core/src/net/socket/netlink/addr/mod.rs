#![short_vis_path::add(netlink)]

mod multicast;

pub(crate) use multicast::GroupIdSet;
pub(in netlink) use multicast::MAX_GROUPS;

use crate::{net::socket::util::SocketAddr, prelude::*};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NetlinkSocketAddr {
    port: PortNum,
    groups: GroupIdSet,
}

impl NetlinkSocketAddr {
    pub(crate) const fn new(port: PortNum, groups: GroupIdSet) -> Self {
        Self { port, groups }
    }

    pub(in netlink) const fn new_unspecified() -> Self {
        Self {
            port: UNSPECIFIED_PORT,
            groups: GroupIdSet::new_empty(),
        }
    }

    pub(crate) const fn port(&self) -> PortNum {
        self.port
    }

    pub(crate) const fn groups(&self) -> GroupIdSet {
        self.groups
    }

    pub(in netlink) fn add_groups(&mut self, groups: GroupIdSet) {
        self.groups.add_groups(groups);
    }
}

impl TryFrom<SocketAddr> for NetlinkSocketAddr {
    type Error = Error;

    fn try_from(value: SocketAddr) -> Result<Self> {
        match value {
            SocketAddr::Netlink(addr) => Ok(addr),
            _ => Err(errno!(
                EAFNOSUPPORT,
                "the address is in an unsupported address family"
            )),
        }
    }
}

impl From<NetlinkSocketAddr> for SocketAddr {
    fn from(value: NetlinkSocketAddr) -> Self {
        SocketAddr::Netlink(value)
    }
}

pub(in netlink) type NetlinkProtocolId = u32;
pub(in netlink) type PortNum = u32;

pub(in netlink) const UNSPECIFIED_PORT: PortNum = 0;
