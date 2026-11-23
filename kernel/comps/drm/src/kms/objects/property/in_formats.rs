use alloc::{boxed::Box, vec::Vec};

use ostd_pod::IntoBytes;

use crate::kms::pixel_format::DrmPixelFormat;

const FORMAT_BLOB_CURRENT: u32 = 1;

#[derive(Debug)]
pub(crate) struct DrmInFormats {
    formats: Box<[DrmPixelFormat]>,
}

impl DrmInFormats {
    pub(crate) fn new(formats: Box<[DrmPixelFormat]>) -> Self {
        Self { formats }
    }

    pub(crate) fn formats(&self) -> &[DrmPixelFormat] {
        &self.formats
    }

    pub(crate) fn encode_blob(&self) -> Box<[u8]> {
        let formats_offset = size_of::<DrmFormatModifierBlobHeader>();
        let modifiers_offset = formats_offset + self.formats.len() * size_of::<u32>();
        let mut data = Vec::with_capacity(modifiers_offset);

        let header = DrmFormatModifierBlobHeader {
            version: FORMAT_BLOB_CURRENT,
            flags: 0,
            count_formats: self.formats.len() as u32,
            formats_offset: formats_offset as u32,
            count_modifiers: 0,
            modifiers_offset: modifiers_offset as u32,
        };
        data.extend_from_slice(header.as_bytes());

        for format in &self.formats {
            data.extend_from_slice((*format as u32).as_bytes());
        }

        data.into_boxed_slice()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct DrmFormatModifierBlobHeader {
    version: u32,
    flags: u32,
    count_formats: u32,
    formats_offset: u32,
    count_modifiers: u32,
    modifiers_offset: u32,
}
