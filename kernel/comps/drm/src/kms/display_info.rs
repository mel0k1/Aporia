use aster_core::prelude::*;
use int_to_c_enum::TryFromInt;

use crate::utils::DrmSize;

#[derive(Clone, Copy, Debug)]
pub struct DrmDisplayInfo {
    mm_width: u32,
    mm_height: u32,
    subpixel_order: SubpixelOrder,
}

impl DrmDisplayInfo {
    pub fn from_dpi(resolution: DrmSize, dpi: u32, subpixel_order: SubpixelOrder) -> Result<Self> {
        if dpi == 0 {
            return Err(errno!(EINVAL, "the display DPI must be nonzero"));
        }

        fn pixels_to_mm(pixels: u32, dpi: u32) -> Result<u32> {
            const MILLIMETERS_PER_INCH_TIMES_TEN: u64 = 254;
            const SCALE: u64 = 10;

            let millimeters =
                u64::from(pixels) * MILLIMETERS_PER_INCH_TIMES_TEN / (u64::from(dpi) * SCALE);

            u32::try_from(millimeters)
                .map_err(|_| errno!(EOVERFLOW, "the calculated display dimension is too large",))
        }

        Ok(Self {
            mm_width: pixels_to_mm(resolution.width(), dpi)?,
            mm_height: pixels_to_mm(resolution.height(), dpi)?,
            subpixel_order,
        })
    }

    pub fn mm_width(&self) -> u32 {
        self.mm_width
    }

    pub fn mm_height(&self) -> u32 {
        self.mm_height
    }

    pub fn subpixel_order(&self) -> u32 {
        self.subpixel_order as u32
    }
}

impl Default for DrmDisplayInfo {
    fn default() -> Self {
        Self {
            mm_width: 0,
            mm_height: 0,
            subpixel_order: SubpixelOrder::Unknown,
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub enum SubpixelOrder {
    Unknown = 0,
    HorizontalRgb = 1,
    HorizontalBgr = 2,
    VerticalRgb = 3,
    VerticalBgr = 4,
    None = 5,
}
