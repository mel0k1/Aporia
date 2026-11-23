use alloc::sync::Arc;
use core::fmt::Debug;

use aster_core::prelude::*;

use crate::{
    gem::object::DrmGemObject,
    kms::{objects::KmsObjectId, pixel_format::DrmPixelFormat},
    utils::DrmSize,
};

#[derive(Debug)]
pub struct DrmFramebuffer {
    id: KmsObjectId,
    size: DrmSize,
    pixel_format: DrmPixelFormat,
    #[expect(dead_code)]
    flags: DrmFramebufferFlags,
    pitch: u32,
    offset: u32,
    #[expect(dead_code)]
    modifier: u64,
    gem_object: Arc<DrmGemObject>,
}

impl DrmFramebuffer {
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: KmsObjectId,
        size: DrmSize,
        pixel_format: DrmPixelFormat,
        flags: DrmFramebufferFlags,
        pitch: u32,
        offset: u32,
        modifier: u64,
        gem_object: Arc<DrmGemObject>,
    ) -> Result<Self> {
        if size.is_empty() {
            return Err(errno!(EINVAL, "the DRM framebuffer size must not be empty"));
        }

        let row_size = (size.width() as usize)
            .checked_mul(pixel_format.bytes_per_pixel())
            .ok_or_else(|| errno!(EINVAL, "the DRM framebuffer row size overflows usize",))?;
        if (pitch as usize) < row_size {
            return Err(errno!(
                EINVAL,
                "the DRM framebuffer pitch is smaller than one pixel row"
            ));
        }

        let required_size = ((size.height() - 1) as usize)
            .checked_mul(pitch as usize)
            .and_then(|size| size.checked_add(offset as usize))
            .and_then(|size| size.checked_add(row_size))
            .ok_or_else(|| errno!(EINVAL, "the DRM framebuffer required size overflows usize",))?;

        if required_size > gem_object.size() {
            return Err(errno!(EINVAL, "the DRM framebuffer exceeds its GEM object"));
        }

        Ok(Self {
            id,
            size,
            pixel_format,
            flags,
            pitch,
            offset,
            modifier,
            gem_object,
        })
    }

    pub(crate) fn id(&self) -> KmsObjectId {
        self.id
    }

    pub fn size(&self) -> DrmSize {
        self.size
    }

    pub fn pixel_format(&self) -> DrmPixelFormat {
        self.pixel_format
    }

    pub fn pitch(&self) -> u32 {
        self.pitch
    }

    pub fn offset(&self) -> u32 {
        self.offset
    }

    pub fn gem_object(&self) -> &DrmGemObject {
        &self.gem_object
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct DrmFramebufferFlags: u32 {
        const INTERLACED = 1 << 0;
        const MODIFIERS  = 1 << 1;
    }
}
