#![short_vis_path::add(netlink)]

mod attr;
mod segment;

pub(super) use attr::{
    AddrAttrClass, LinkAttrClass,
    addr::{AddrAttr, AddrProtocol},
    link::LinkAttr,
};
pub(super) use segment::{
    RtnlSegment,
    addr::{AddrMessageFlags, AddrSegment, CIfaddrMsg, RtScope},
    link::{CIfinfoMsg, LinkSegment},
};

use crate::net::socket::netlink::message::Message;

pub(in netlink) type RtnlMessage = Message<RtnlSegment>;
