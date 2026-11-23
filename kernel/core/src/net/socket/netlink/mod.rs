mod addr;
mod common;
mod kobject_uevent;
mod message;
mod options;
mod receiver;
mod route;
mod table;

pub(crate) use addr::{GroupIdSet, NetlinkSocketAddr};
pub(crate) use kobject_uevent::{NetlinkUeventSocket, broadcast_uevent};
pub(crate) use options::{AddMembership, DropMembership, StrictCheck};
pub(super) use receiver::NETLINK_DEFAULT_BUF_SIZE;
pub(crate) use route::NetlinkRouteSocket;
pub(crate) use table::{StandardNetlinkProtocol, is_valid_protocol};

pub(in crate::net) fn init() {
    table::init();
}
