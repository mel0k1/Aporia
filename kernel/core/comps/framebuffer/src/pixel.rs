#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pixel {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Grayscale8,
    Rgb565,
    Rgb888,
    BgrReserved,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderedPixel {
    buf: [u8; 4],
    len: u8,
}

impl Pixel {
    pub fn render(&self, format: PixelFormat) -> RenderedPixel {
        let mut buf = [0; 4];
        match format {
            PixelFormat::Grayscale8 => {
                let red_weight = 77 * self.red as u16;
                let green_weight = 150 * self.green as u16;
                let blue_weight = 29 * self.blue as u16;
                let grayscale = (red_weight + green_weight + blue_weight) >> 8;
                buf[0] = grayscale as u8;
                RenderedPixel { buf, len: 1 }
            }
            PixelFormat::Rgb565 => {
                let r = (self.red >> 3) as u16;
                let g = (self.green >> 2) as u16;
                let b = (self.blue >> 3) as u16;
                let rgb565 = (r << 11) | (g << 5) | b;
                buf[0..2].copy_from_slice(&rgb565.to_be_bytes());
                RenderedPixel { buf, len: 2 }
            }
            PixelFormat::Rgb888 => {
                buf[0] = self.red;
                buf[1] = self.green;
                buf[2] = self.blue;
                RenderedPixel { buf, len: 3 }
            }
            PixelFormat::BgrReserved => {
                buf[0] = self.blue;
                buf[1] = self.green;
                buf[2] = self.red;
                RenderedPixel { buf, len: 4 }
            }
        }
    }
}

impl PixelFormat {
    pub fn nbytes(&self) -> usize {
        match self {
            PixelFormat::Grayscale8 => 1,
            PixelFormat::Rgb565 => 2,
            PixelFormat::Rgb888 => 3,
            PixelFormat::BgrReserved => 4,
        }
    }
}

impl RenderedPixel {
    pub fn nbytes(&self) -> usize {
        self.len as usize
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..self.nbytes()]
    }
}

impl Pixel {
    pub const WHITE: Pixel = Pixel {
        red: 0xFF,
        green: 0xFF,
        blue: 0xFF,
    };
    pub const BLACK: Pixel = Pixel {
        red: 0x00,
        green: 0x00,
        blue: 0x00,
    };
}
