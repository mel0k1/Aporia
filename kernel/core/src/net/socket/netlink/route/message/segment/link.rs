use super::legacy::CRtGenMsg;
use crate::{
    net::socket::netlink::{
        message::{SegmentBody, SegmentCommon},
        route::message::attr::link::LinkAttr,
    },
    prelude::*,
};

pub(in crate::net::socket::netlink::route) type LinkSegment = SegmentCommon<CIfinfoMsg, LinkAttr>;

impl SegmentBody for CIfinfoMsg {
    type CLegacyType = CRtGenMsg;
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct CIfinfoMsg {
    pub family: u8,
    pub _pad: u8,
    pub type_: u16,
    pub index: u32,
    pub flags: u32,
    pub change: u32,
}
