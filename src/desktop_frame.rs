//! Bounded screencopy conversion independent of Wayland and native GPU drivers.
use crate::Dimensions;
use std::io;
pub const MAX_CAPTURE_BYTES: usize = 128 * 1024 * 1024;
#[derive(Clone, Copy, Debug)]
pub enum PixelFormat {
    Rgb,
    Bgr,
    Rgb10,
    Bgr10,
}
#[derive(Clone, Copy, Debug)]
pub struct CaptureLayout {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub format: PixelFormat,
    pub inverted: bool,
}
impl CaptureLayout {
    pub fn byte_len(self) -> io::Result<usize> {
        let len = self
            .stride
            .checked_mul(self.height)
            .ok_or_else(|| bad("capture overflow"))?;
        if self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 8192
            || self.stride < self.width * 4
            || !self.stride.is_multiple_of(4)
            || len > MAX_CAPTURE_BYTES
        {
            return Err(bad("capture size exceeds limits"));
        }
        Ok(len)
    }
    pub fn convert(self, bytes: &[u8], bound: Dimensions) -> io::Result<(Dimensions, Vec<u32>)> {
        let len = self.byte_len()?;
        if bytes.len() != len {
            return Err(bad("wrong capture mapping length"));
        }
        bound.validate()?;
        let scale = (bound.width as f64 / self.width as f64)
            .min(bound.height as f64 / self.height as f64)
            .min(1.0);
        let size = Dimensions::new(
            (self.width as f64 * scale).floor().max(1.0) as usize,
            (self.height as f64 * scale).floor().max(1.0) as usize,
        )?;
        let mut pixels = Vec::with_capacity(size.pixel_count());
        for y in 0..size.height as usize {
            let mut source_y = y * self.height / size.height as usize;
            if self.inverted {
                source_y = self.height - 1 - source_y;
            }
            for x in 0..size.width as usize {
                let offset = source_y * self.stride + (x * self.width / size.width as usize) * 4;
                let value = u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
                let pixel = match self.format {
                    PixelFormat::Rgb => value & 0xffffff,
                    PixelFormat::Bgr => {
                        ((value & 255) << 16) | (value & 0xff00) | ((value >> 16) & 255)
                    }
                    PixelFormat::Rgb10 | PixelFormat::Bgr10 => {
                        let r = ((value >> 20) & 1023) * 255 / 1023;
                        let g = ((value >> 10) & 1023) * 255 / 1023;
                        let b = (value & 1023) * 255 / 1023;
                        if matches!(self.format, PixelFormat::Rgb10) {
                            r << 16 | g << 8 | b
                        } else {
                            b << 16 | g << 8 | r
                        }
                    }
                };
                pixels.push(pixel);
            }
        }
        Ok((size, pixels))
    }
}
fn bad(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, text)
}
