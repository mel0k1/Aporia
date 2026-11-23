#![short_vis_path::add(netlink)]

use align_ext::AlignExt;

use crate::{net::socket::netlink::message::NLMSG_ALIGN, prelude::*, util::MultiRead};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct CMsgSegHdr {
    pub len: u32,
    pub type_: u16,
    pub flags: u16,
    pub seq: u32,
    pub pid: u32,
}

impl CMsgSegHdr {
    pub(in netlink) fn calc_payload_len(&self, reader: &dyn MultiRead) -> Result<usize> {
        let payload_len = (self.len as usize)
            .checked_sub(size_of::<Self>())
            .ok_or_else(|| errno!(EINVAL, "the message length is too small"))?;

        let reader_len = reader.sum_lens();
        if reader_len < payload_len {
            return Err(errno!(EINVAL, "the reader length is too small"));
        }

        Ok(payload_len)
    }

    pub(in netlink) fn calc_payload_len_with_padding(
        &self,
        reader: &dyn MultiRead,
    ) -> Result<usize> {
        let payload_len = self.calc_payload_len(reader)?;
        Ok(payload_len.align_up(NLMSG_ALIGN).min(reader.sum_lens()))
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(in crate::net::socket::netlink) struct SegHdrCommonFlags: u16 {
        const REQUEST       = 0x01;
        const MULTI         = 0x02;
        const ACK           = 0x04;
        const ECHO          = 0x08;
        const DUMP_INTR     = 0x10;
        const DUMP_FILTERED = 0x20;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(in crate::net::socket::netlink) struct GetRequestFlags: u16 {
        const ROOT   = 0x100;
        const MATCH  = 0x200;
        const ATOMIC = 0x400;
        const DUMP   = Self::ROOT.bits() | Self::MATCH.bits();
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct NewRequestFlags: u16 {
        const REPLACE = 0x100;
        const EXCL    = 0x200;
        const CREATE  = 0x400;
        const APPEND  = 0x800;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct DeleteRequestFlags: u16 {
        const NONREC = 0x100;
        const BULK   = 0x200;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct AckFlags: u16 {
        const CAPPED   = 0x100;
        const ACK_TLVS = 0x100;
    }
}
