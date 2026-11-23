#![short_vis_path::add(netlink)]

use align_ext::AlignExt;

use super::{ContinueAttrRead, ContinueRead, NLMSG_ALIGN};
use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub(super) mod noattr;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct CAttrHeader {
    len: u16,
    type_: u16,
}

impl CAttrHeader {
    fn from_payload_len(type_: u16, payload_len: usize) -> Self {
        let total_len = payload_len + size_of::<Self>();
        debug_assert!(total_len <= u16::MAX as usize);

        Self {
            len: total_len as u16,
            type_,
        }
    }

    pub(in netlink) fn type_(&self) -> u16 {
        self.type_ & ATTRIBUTE_TYPE_MASK
    }

    pub(in netlink) fn payload_len(&self) -> usize {
        self.len as usize - size_of::<Self>()
    }

    fn total_len(&self) -> usize {
        self.len as usize
    }

    fn total_len_with_padding(&self) -> usize {
        (self.len as usize).align_up(NLMSG_ALIGN)
    }

    fn padding_len(&self) -> usize {
        self.total_len_with_padding() - self.total_len()
    }
}

const IS_NESTED_MASK: u16 = 1u16 << 15;
const IS_NET_BYTEORDER_MASK: u16 = 1u16 << 14;
const ATTRIBUTE_TYPE_MASK: u16 = !(IS_NESTED_MASK | IS_NET_BYTEORDER_MASK);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AttrValidation {
    Strict,
    LegacyQuery,
    LegacyDump,
}

pub(crate) trait Attribute: Debug + Send + Sync {
    fn type_(&self) -> u16;

    fn payload_as_bytes(&self) -> &[u8];

    fn total_len_with_padding(&self) -> usize {
        const DUMMY_TYPE: u16 = 0;

        CAttrHeader::from_payload_len(DUMMY_TYPE, self.payload_as_bytes().len())
            .total_len_with_padding()
    }

    fn read_from(
        header: &CAttrHeader,
        reader: &mut dyn MultiRead,
    ) -> Result<ContinueAttrRead<Self>>
    where
        Self: Sized;

    fn read_all_from(
        reader: &mut dyn MultiRead,
        mut total_len: usize,
        validation: AttrValidation,
    ) -> Result<ContinueRead<Vec<Self>>>
    where
        Self: Sized,
    {
        let mut res = Vec::new();

        let mut error = None;
        let mut should_report_error = validation == AttrValidation::Strict;
        while total_len > 0 {
            if total_len < size_of::<CAttrHeader>() {
                error = Some(errno!(EINVAL, "the reader length is too small",));
                break;
            }

            let header = reader.read_val_opt::<CAttrHeader>()?.unwrap();
            total_len -= size_of::<CAttrHeader>();
            if header.total_len() < size_of::<CAttrHeader>() {
                error = Some(errno!(EINVAL, "the attribute length is too small",));
                break;
            }

            if header.payload_len() > total_len {
                error = Some(errno!(EINVAL, "the reader size is too small",));
                break;
            }
            total_len -= header.payload_len();

            match Self::read_from(&header, reader)? {
                ContinueAttrRead::Parsed(attr) => res.push(attr),
                ContinueAttrRead::SkippedUnsupported => {
                    if validation == AttrValidation::Strict {
                        error = Some(errno!(EINVAL, "the attribute is not supported",));
                        break;
                    }
                }
                ContinueAttrRead::SkippedInvalid(err) => {
                    error = Some(err);
                    should_report_error = validation != AttrValidation::LegacyDump;
                    break;
                }
            }

            let padding_len = total_len.min(header.padding_len());
            reader.skip_some(padding_len);
            total_len -= padding_len;
        }

        reader.skip_some(total_len);
        if should_report_error && let Some(err) = error {
            return Ok(ContinueRead::SkippedInvalid(err));
        }

        Ok(ContinueRead::Parsed(res))
    }

    fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        let type_ = self.type_();
        let payload = self.payload_as_bytes();

        let header = CAttrHeader::from_payload_len(type_, payload.len());
        writer.write_val_trunc(&header)?;
        writer.write(&mut VmReader::from(payload))?;

        let padding_len = header.padding_len();
        writer.skip_some(padding_len);

        Ok(())
    }
}
