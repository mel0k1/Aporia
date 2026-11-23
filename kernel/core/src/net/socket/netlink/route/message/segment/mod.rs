pub(super) mod addr;
mod legacy;
pub(super) mod link;
mod route;

use addr::AddrSegment;
use link::LinkSegment;

use crate::{
    net::socket::netlink::message::{
        CMsgSegHdr, CSegmentType, ContinueRead, DoneSegment, ErrorSegment, ProtocolSegment,
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

#[derive(Debug)]
pub(crate) enum RtnlSegment {
    NewLink(LinkSegment),
    GetLink(LinkSegment),
    NewAddr(AddrSegment),
    GetAddr(AddrSegment),
    Done(DoneSegment),
    Error(ErrorSegment),
}

impl ProtocolSegment for RtnlSegment {
    fn header(&self) -> &CMsgSegHdr {
        match self {
            RtnlSegment::NewLink(link_segment) | RtnlSegment::GetLink(link_segment) => {
                link_segment.header()
            }
            RtnlSegment::NewAddr(addr_segment) | RtnlSegment::GetAddr(addr_segment) => {
                addr_segment.header()
            }
            RtnlSegment::Done(done_segment) => done_segment.header(),
            RtnlSegment::Error(error_segment) => error_segment.header(),
        }
    }

    fn header_mut(&mut self) -> &mut CMsgSegHdr {
        match self {
            RtnlSegment::NewLink(link_segment) | RtnlSegment::GetLink(link_segment) => {
                link_segment.header_mut()
            }
            RtnlSegment::NewAddr(addr_segment) | RtnlSegment::GetAddr(addr_segment) => {
                addr_segment.header_mut()
            }
            RtnlSegment::Done(done_segment) => done_segment.header_mut(),
            RtnlSegment::Error(error_segment) => error_segment.header_mut(),
        }
    }

    fn read_from(
        reader: &mut dyn MultiRead,
        strict_check: bool,
    ) -> Result<ContinueRead<Self, ErrorSegment>> {
        let header = reader
            .read_val_opt::<CMsgSegHdr>()?
            .ok_or_else(|| errno!(EINVAL, "the reader length is too small"))?;

        let segment = match CSegmentType::try_from(header.type_) {
            Ok(CSegmentType::GETLINK) => {
                LinkSegment::read_from(&header, reader, strict_check)?.map(RtnlSegment::GetLink)
            }
            Ok(CSegmentType::GETADDR) => {
                AddrSegment::read_from(&header, reader, strict_check)?.map(RtnlSegment::GetAddr)
            }
            _ => {
                let payload_len = header.calc_payload_len_with_padding(reader)?;
                reader.skip_some(payload_len);
                ContinueRead::SkippedInvalid(errno!(
                    EOPNOTSUPP,
                    "the segment type is not supported",
                ))
            }
        };

        Ok(segment.map_err(|error| ErrorSegment::new_from_request(&header, Some(error))))
    }

    fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        match self {
            RtnlSegment::NewLink(link_segment) => link_segment.write_to(writer)?,
            RtnlSegment::NewAddr(addr_segment) => addr_segment.write_to(writer)?,
            RtnlSegment::Done(done_segment) => done_segment.write_to(writer)?,
            RtnlSegment::Error(error_segment) => error_segment.write_to(writer)?,
            RtnlSegment::GetAddr(_) | RtnlSegment::GetLink(_) => {
                unreachable!("kernel should not write get requests to user space");
            }
        }
        Ok(())
    }
}
