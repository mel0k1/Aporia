mod attr;
mod result;
mod segment;

pub(super) use attr::{Attribute, CAttrHeader, noattr::NoAttr};
pub(super) use result::{ContinueAttrRead, ContinueRead};
pub(super) use segment::{
    CSegmentType, SegmentBody,
    ack::{DoneSegment, ErrorSegment},
    common::SegmentCommon,
    header::{CMsgSegHdr, GetRequestFlags, SegHdrCommonFlags},
};

use super::receiver::QueueableMessage;
use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

#[derive(Debug)]
pub(crate) struct Message<T> {
    segments: Vec<T>,
}

impl<T> Message<T> {
    pub(super) const fn new(segments: Vec<T>) -> Self {
        Self { segments }
    }
}

impl<T: ProtocolSegment> Message<T> {
    pub(super) fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        for segment in self.segments.iter() {
            segment.write_to(writer)?;
        }

        Ok(())
    }
}

impl<T: ProtocolSegment> QueueableMessage for Message<T> {
    fn total_len(&self) -> usize {
        self.segments
            .iter()
            .map(|segment| segment.header().len as usize)
            .sum()
    }
}

pub(crate) trait ProtocolSegment: Sized {
    fn header(&self) -> &CMsgSegHdr;
    fn header_mut(&mut self) -> &mut CMsgSegHdr;

    fn read_from(
        reader: &mut dyn MultiRead,
        strict_check: bool,
    ) -> Result<ContinueRead<Self, ErrorSegment>>;

    fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()>;
}

pub(super) const NLMSG_ALIGN: usize = 4;
