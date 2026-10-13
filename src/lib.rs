//! Versioned, bounded transport and lossless dirty-tile reconstruction.
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
pub const VERSION: u16 = 3;
pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 400;
pub const TILE: usize = 32;
// JSON byte arrays need up to four bytes per compressed byte. This ceiling
// accommodates a bounded full-HD keyframe, including incompressible content.
pub const MAX_PACKET: usize = 40 * 1024 * 1024;
pub const MAX_DIMENSION: usize = 2048;
pub const MAX_PIXELS: usize = 2_097_152;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u16,
    pub height: u16,
}
impl Default for Dimensions {
    fn default() -> Self {
        Self {
            width: WIDTH as u16,
            height: HEIGHT as u16,
        }
    }
}
impl Dimensions {
    pub fn new(width: usize, height: usize) -> io::Result<Self> {
        if width == 0
            || height == 0
            || width > MAX_DIMENSION
            || height > MAX_DIMENSION
            || width * height > MAX_PIXELS
        {
            return Err(invalid("unsupported framebuffer dimensions"));
        }
        Ok(Self {
            width: width as u16,
            height: height as u16,
        })
    }
    pub fn validate(self) -> io::Result<Self> {
        Self::new(self.width as usize, self.height as usize)
    }
    pub fn pixel_count(self) -> usize {
        self.width as usize * self.height as usize
    }
    /// Fit a native window into transport limits, preserving aspect ratio.
    pub fn fit(width: usize, height: usize) -> Self {
        let w = width.max(1) as f64;
        let h = height.max(1) as f64;
        let scale = (MAX_DIMENSION as f64 / w)
            .min(MAX_DIMENSION as f64 / h)
            .min((MAX_PIXELS as f64 / (w * h)).sqrt())
            .min(1.0);
        Self::new(
            (w * scale).floor().max(1.0) as usize,
            (h * scale).floor().max(1.0) as usize,
        )
        .unwrap()
    }
}
fn invalid(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
impl std::str::FromStr for Dimensions {
    type Err = io::Error;
    fn from_str(value: &str) -> io::Result<Self> {
        let (width, height) = value
            .split_once('x')
            .ok_or_else(|| invalid("size must be WIDTHxHEIGHT"))?;
        Self::new(
            width.parse().map_err(|_| invalid("invalid width"))?,
            height.parse().map_err(|_| invalid("invalid height"))?,
        )
    }
}
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum Message {
    Hello {
        version: u16,
    },
    Frame {
        sequence: u64,
        size: Dimensions,
        reset: bool,
        tiles: Vec<Tile>,
    },
    Pointer {
        x: u16,
        y: u16,
        pressed: bool,
    },
    Resize {
        size: Dimensions,
    },
    /// Full held-key snapshot. Evdev codes (not XKB codes); US layout initially.
    Keys {
        held: Vec<u16>,
    },
    Buttons {
        right: bool,
        middle: bool,
    },
    Scroll {
        horizontal: i16,
        vertical: i16,
    },
    Focus {
        active: bool,
    },
}
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Tile {
    pub x: usize,
    pub y: usize,
    pub data: Vec<u8>,
}
pub fn write_message(mut w: impl Write, message: &Message) -> io::Result<usize> {
    let data = serde_json::to_vec(message)?;
    if data.len() > MAX_PACKET {
        return Err(invalid("packet too large"));
    }
    w.write_all(&(data.len() as u32).to_be_bytes())?;
    w.write_all(&data)?;
    Ok(data.len() + 4)
}
pub fn read_message(r: impl Read) -> io::Result<Message> {
    read_message_limit(r, MAX_PACKET)
}
/// Apply a smaller limit for control/input channels before allocating the body.
pub fn read_message_limit(mut r: impl Read, limit: usize) -> io::Result<Message> {
    let mut header = [0; 4];
    r.read_exact(&mut header)?;
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > limit.min(MAX_PACKET) {
        return Err(invalid("invalid packet length"));
    }
    let mut data = vec![0; len];
    r.read_exact(&mut data)?;
    serde_json::from_slice(&data).map_err(Into::into)
}
pub fn handshake(stream: &mut std::net::TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(10)))?;
    write_message(&mut *stream, &Message::Hello { version: VERSION })?;
    if read_message_limit(&mut *stream, 4096)? != (Message::Hello { version: VERSION }) {
        return Err(invalid("unsupported protocol version"));
    }
    stream.set_read_timeout(None)
}
#[derive(Default)]
pub struct Encoder {
    previous: Option<Vec<u32>>,
    size: Dimensions,
    sequence: u64,
}
impl Encoder {
    /// Returns None for an unchanged frame: idle content sends zero bytes.
    pub fn encode(&mut self, pixels: &[u32]) -> io::Result<Option<Message>> {
        self.encode_sized(Dimensions::default(), pixels)
    }
    pub fn encode_sized(
        &mut self,
        size: Dimensions,
        pixels: &[u32],
    ) -> io::Result<Option<Message>> {
        let size = size.validate()?;
        let (width, height) = (size.width as usize, size.height as usize);
        let reset = self.previous.is_none() || self.size != size;
        if pixels.len() != size.pixel_count() {
            return Err(invalid("wrong frame size"));
        }
        let mut tiles = Vec::new();
        for y in (0..height).step_by(TILE) {
            for x in (0..width).step_by(TILE) {
                let h = TILE.min(height - y);
                let w = TILE.min(width - x);
                let changed = reset
                    || self.previous.as_ref().is_none_or(|old| {
                        (y..y + h).any(|row| {
                            old[row * width + x..row * width + x + w]
                                != pixels[row * width + x..row * width + x + w]
                        })
                    });
                if changed {
                    let raw: Vec<u8> = (y..y + h)
                        .flat_map(|row| {
                            pixels[row * width + x..row * width + x + w]
                                .iter()
                                .flat_map(|p| p.to_le_bytes())
                        })
                        .collect();
                    tiles.push(Tile {
                        x,
                        y,
                        data: lz4_flex::block::compress(&raw),
                    });
                }
            }
        }
        if tiles.is_empty() {
            return Ok(None);
        }
        self.previous = Some(pixels.to_vec());
        self.size = size;
        self.sequence += 1;
        Ok(Some(Message::Frame {
            sequence: self.sequence,
            size,
            reset,
            tiles,
        }))
    }
}
pub struct Decoder {
    pub pixels: Vec<u32>,
    pub size: Dimensions,
    sequence: u64,
}
impl Default for Decoder {
    fn default() -> Self {
        Self {
            pixels: vec![0; WIDTH * HEIGHT],
            size: Dimensions::default(),
            sequence: 0,
        }
    }
}
impl Decoder {
    /// Validate the entire update before committing it, including exact LZ4 output size.
    pub fn apply(&mut self, message: Message) -> io::Result<()> {
        let Message::Frame {
            sequence,
            size,
            reset,
            tiles,
        } = message
        else {
            return Err(invalid("expected frame"));
        };
        let size = size.validate()?;
        let (width, height) = (size.width as usize, size.height as usize);
        if (!reset && (self.sequence == 0 || size != self.size))
            || Some(sequence) != self.sequence.checked_add(1)
            || tiles.is_empty()
            || tiles.len() > width.div_ceil(TILE) * height.div_ceil(TILE)
        {
            return Err(invalid("invalid sequence or tile count"));
        }
        let mut next = if reset {
            vec![0; size.pixel_count()]
        } else {
            self.pixels.clone()
        };
        let mut seen = std::collections::HashSet::new();
        for tile in tiles {
            if tile.x >= width
                || tile.y >= height
                || tile.x % TILE != 0
                || tile.y % TILE != 0
                || !seen.insert((tile.x, tile.y))
            {
                return Err(invalid("invalid tile coordinates"));
            }
            let w = TILE.min(width - tile.x);
            let h = TILE.min(height - tile.y);
            let mut raw = vec![0; w * h * 4];
            let n = lz4_flex::block::decompress_into(&tile.data, &mut raw)
                .map_err(|_| invalid("bad LZ4 tile"))?;
            if n != raw.len() {
                return Err(invalid("wrong tile size"));
            }
            for row in 0..h {
                for col in 0..w {
                    let i = (row * w + col) * 4;
                    next[(tile.y + row) * width + tile.x + col] =
                        u32::from_le_bytes(raw[i..i + 4].try_into().unwrap());
                }
            }
        }
        if reset && seen.len() != width.div_ceil(TILE) * height.div_ceil(TILE) {
            return Err(invalid("incomplete initial frame"));
        }
        self.pixels = next;
        self.sequence = sequence;
        self.size = size;
        Ok(())
    }
}
/// Synthetic UI: static rows plus a pointer-controlled colored square.
pub fn demo_frame(x: usize, y: usize, pressed: bool) -> Vec<u32> {
    let mut pixels = vec![0x18202c; WIDTH * HEIGHT];
    for row in 0..HEIGHT {
        for col in 0..WIDTH {
            if row % 24 < 3 && col > 32 && col < 480 {
                pixels[row * WIDTH + col] = 0x697c91;
            }
            if col.abs_diff(x) < 12 && row.abs_diff(y) < 12 {
                pixels[row * WIDTH + col] = if pressed { 0xffa040 } else { 0x40d0a0 };
            }
        }
    }
    pixels
}

#[cfg(all(target_os = "linux", feature = "compositor"))]
pub mod compositor;
pub mod remote;
#[cfg(feature = "viewer")]
pub mod viewer_input;
