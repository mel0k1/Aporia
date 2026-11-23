use align_ext::AlignExt;
use header::CMsgSegHdr;

use super::{ContinueRead, NLMSG_ALIGN};
use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub(super) mod ack;
pub(super) mod common;
pub(super) mod header;

pub(crate) trait SegmentBody: Pod + From<Self::CLegacyType> {
    type CLegacyType: Pod = Self;

    fn read_from(
        header: &CMsgSegHdr,
        reader: &mut dyn MultiRead,
        strict_check: bool,
    ) -> Result<ContinueRead<(Self, usize)>> {
        let mut remaining_len = header.calc_payload_len(reader)?;

        let (body, padding_len) = if remaining_len >= size_of::<Self>() {
            let body = reader.read_val_opt::<Self>()?.unwrap();
            remaining_len -= size_of_val(&body);

            (body, Self::padding_len())
        } else if remaining_len >= size_of::<Self::CLegacyType>() && !strict_check {
            let legacy = reader.read_val_opt::<Self::CLegacyType>()?.unwrap();
            remaining_len -= size_of_val(&legacy);

            (Self::from(legacy), Self::legacy_padding_len())
        } else {
            reader.skip_some(remaining_len);
            return Ok(ContinueRead::SkippedInvalid(errno!(
                EINVAL,
                "the message length is too small",
            )));
        };

        let padding_len = padding_len.min(remaining_len);
        reader.skip_some(padding_len);
        remaining_len -= padding_len;

        Ok(ContinueRead::Parsed((body, remaining_len)))
    }

    fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        writer.write_val_trunc(self)?;

        let padding_len = Self::padding_len();
        writer.skip_some(padding_len);

        Ok(())
    }

    fn padding_len() -> usize {
        let payload_len = size_of::<Self>();
        payload_len.align_up(NLMSG_ALIGN) - payload_len
    }

    fn legacy_padding_len() -> usize {
        let payload_len = size_of::<Self::CLegacyType>();
        payload_len.align_up(NLMSG_ALIGN) - payload_len
    }
}

#[expect(clippy::upper_case_acronyms)]
#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, TryFromInt)]
pub(in crate::net::socket::netlink) enum CSegmentType {
    NOOP = 1,
    ERROR = 2,
    DONE = 3,
    OVERRUN = 4,

    NEWLINK = 16,
    DELLINK = 17,
    GETLINK = 18,
    SETLINK = 19,

    NEWADDR = 20,
    DELADDR = 21,
    GETADDR = 22,

    NEWROUTE = 24,
    DELROUTE = 25,
    GETROUTE = 26,
}
