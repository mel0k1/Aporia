use alloc::{sync::Arc, vec::Vec};

use align_ext::AlignExt;
use ostd::{
    Error, Result,
    boot::boot_info,
    io::IoMem,
    mm::{CachePolicy, HasSize, PAGE_SIZE, VmIo},
    sync::Mutex,
};
use spin::Once;

use crate::pixel::{Pixel, PixelFormat, RenderedPixel};

pub const MAX_CMAP_SIZE: usize = 256;

#[derive(Debug)]
pub struct FrameBuffer {
    io_mem: IoMem,
    width: usize,
    height: usize,
    line_size: usize,
    pixel_format: PixelFormat,
    cmap: Mutex<FbCmap>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ColorMapEntry {
    pub red: u16,
    pub green: u16,
    pub blue: u16,
    pub transp: u16,
}

#[derive(Clone, Debug)]
struct FbCmap {
    entries: Vec<ColorMapEntry>,
}

pub static FRAMEBUFFER: Once<Arc<FrameBuffer>> = Once::new();

pub(crate) fn init() {
    let Some(framebuffer_arg) = boot_info().framebuffer_arg else {
        ostd::warn!("Framebuffer not found");
        return;
    };

    if framebuffer_arg.address == 0 {
        ostd::error!("Framebuffer address is zero");
        return;
    }

    if framebuffer_arg.address % PAGE_SIZE != 0 {
        ostd::error!("Framebuffer address is not page-aligned");
        return;
    }

    let pixel_format = match framebuffer_arg.bpp {
        8 => PixelFormat::Grayscale8,
        16 => PixelFormat::Rgb565,
        24 => PixelFormat::Rgb888,
        32 => PixelFormat::BgrReserved,
        _ => {
            ostd::error!(
                "Unsupported framebuffer pixel format: {} bpp",
                framebuffer_arg.bpp
            );
            return;
        }
    };

    let framebuffer = {
        let line_size = framebuffer_arg
            .width
            .checked_mul(pixel_format.nbytes())
            .unwrap();
        let fb_size = framebuffer_arg
            .height
            .checked_mul(line_size)
            .unwrap()
            .align_up(PAGE_SIZE);

        let fb_base = framebuffer_arg.address;
        let io_mem = IoMem::acquire_with_cache_policy(
            fb_base..fb_base.checked_add(fb_size).unwrap(),
            CachePolicy::WriteCombining,
        )
        .unwrap();

        let default_cmap = FbCmap {
            entries: Vec::new(),
        };

        FrameBuffer {
            io_mem,
            width: framebuffer_arg.width,
            height: framebuffer_arg.height,
            line_size,
            pixel_format,
            cmap: Mutex::new(default_cmap),
        }
    };

    framebuffer.clear();
    FRAMEBUFFER.call_once(|| Arc::new(framebuffer));
}

impl FrameBuffer {
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn line_size(&self) -> usize {
        self.line_size
    }

    pub fn io_mem(&self) -> &IoMem {
        &self.io_mem
    }

    pub fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    pub fn render_pixel(&self, pixel: Pixel) -> RenderedPixel {
        pixel.render(self.pixel_format)
    }

    pub fn calc_offset(&self, x: usize, y: usize) -> PixelOffset<'_> {
        PixelOffset {
            fb: self,
            offset: (x * self.pixel_format.nbytes() + y * self.line_size) as isize,
        }
    }

    pub fn write_pixel_at(&self, offset: PixelOffset, pixel: RenderedPixel) -> Result<()> {
        self.io_mem.write_bytes(offset.as_usize(), pixel.as_slice())
    }

    pub fn write_bytes_at(&self, offset: usize, bytes: &[u8]) -> Result<()> {
        self.io_mem.write_bytes(offset, bytes)
    }

    pub fn clear(&self) {
        let frame = alloc::vec![0u8; self.io_mem().size()];
        self.write_bytes_at(0, &frame).unwrap();
    }

    pub fn set_color_map(&self, start: usize, entries: &[ColorMapEntry]) -> Result<()> {
        if start > MAX_CMAP_SIZE || entries.len() > MAX_CMAP_SIZE - start {
            return Err(Error::InvalidArgs);
        }

        let mut cmap = self.cmap.lock();
        let required_len = start + entries.len();

        if cmap.entries.len() < required_len {
            cmap.entries.resize(
                required_len,
                ColorMapEntry {
                    red: 0,
                    green: 0,
                    blue: 0,
                    transp: 0,
                },
            );
        }

        cmap.entries[start..start + entries.len()].copy_from_slice(entries);

        Ok(())
    }

    pub fn get_color_map(&self, start: usize, len: usize) -> Option<Vec<ColorMapEntry>> {
        let cmap = self.cmap.lock();

        if start >= cmap.entries.len() || len > cmap.entries.len() - start {
            return None;
        }

        Some(cmap.entries[start..start + len].to_vec())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PixelOffset<'a> {
    fb: &'a FrameBuffer,
    offset: isize,
}

impl PixelOffset<'_> {
    pub fn x_add(&mut self, x_delta: isize) {
        let delta = x_delta * self.fb.pixel_format.nbytes() as isize;
        self.offset += delta;
    }

    pub fn y_add(&mut self, y_delta: isize) {
        let delta = y_delta * self.fb.line_size as isize;
        self.offset += delta;
    }

    pub fn as_usize(&self) -> usize {
        self.offset as usize
    }
}
