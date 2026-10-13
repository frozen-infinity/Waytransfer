//! Versioned, bounded transport and lossless dirty-tile reconstruction.
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
pub const VERSION: u16 = 1;
pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 400;
pub const TILE: usize = 32;
pub const MAX_PACKET: usize = 4 * 1024 * 1024;
fn invalid(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum Message {
    Hello { version: u16 },
    Frame { sequence: u64, tiles: Vec<Tile> },
    Pointer { x: u16, y: u16, pressed: bool },
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
pub fn read_message(mut r: impl Read) -> io::Result<Message> {
    let mut header = [0; 4];
    r.read_exact(&mut header)?;
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > MAX_PACKET {
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
    if read_message(&mut *stream)? != (Message::Hello { version: VERSION }) {
        return Err(invalid("unsupported protocol version"));
    }
    stream.set_read_timeout(None)
}
#[derive(Default)]
pub struct Encoder {
    previous: Option<Vec<u32>>,
    sequence: u64,
}
impl Encoder {
    /// Returns None for an unchanged frame: idle content sends zero bytes.
    pub fn encode(&mut self, pixels: &[u32]) -> io::Result<Option<Message>> {
        if pixels.len() != WIDTH * HEIGHT {
            return Err(invalid("wrong frame size"));
        }
        let mut tiles = Vec::new();
        for y in (0..HEIGHT).step_by(TILE) {
            for x in (0..WIDTH).step_by(TILE) {
                let h = TILE.min(HEIGHT - y);
                let w = TILE.min(WIDTH - x);
                let changed = self.previous.as_ref().is_none_or(|old| {
                    (y..y + h).any(|row| {
                        old[row * WIDTH + x..row * WIDTH + x + w]
                            != pixels[row * WIDTH + x..row * WIDTH + x + w]
                    })
                });
                if changed {
                    let raw: Vec<u8> = (y..y + h)
                        .flat_map(|row| {
                            pixels[row * WIDTH + x..row * WIDTH + x + w]
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
        self.sequence += 1;
        Ok(Some(Message::Frame {
            sequence: self.sequence,
            tiles,
        }))
    }
}
pub struct Decoder {
    pub pixels: Vec<u32>,
    sequence: u64,
}
impl Default for Decoder {
    fn default() -> Self {
        Self {
            pixels: vec![0; WIDTH * HEIGHT],
            sequence: 0,
        }
    }
}
impl Decoder {
    /// Validate the entire update before committing it, including exact LZ4 output size.
    pub fn apply(&mut self, message: Message) -> io::Result<()> {
        let Message::Frame { sequence, tiles } = message else {
            return Err(invalid("expected frame"));
        };
        if sequence != self.sequence + 1
            || tiles.is_empty()
            || tiles.len() > WIDTH.div_ceil(TILE) * HEIGHT.div_ceil(TILE)
        {
            return Err(invalid("invalid sequence or tile count"));
        }
        let mut next = self.pixels.clone();
        let mut seen = std::collections::HashSet::new();
        for tile in tiles {
            if tile.x >= WIDTH
                || tile.y >= HEIGHT
                || tile.x % TILE != 0
                || tile.y % TILE != 0
                || !seen.insert((tile.x, tile.y))
            {
                return Err(invalid("invalid tile coordinates"));
            }
            let w = TILE.min(WIDTH - tile.x);
            let h = TILE.min(HEIGHT - tile.y);
            let mut raw = vec![0; w * h * 4];
            let n = lz4_flex::block::decompress_into(&tile.data, &mut raw)
                .map_err(|_| invalid("bad LZ4 tile"))?;
            if n != raw.len() {
                return Err(invalid("wrong tile size"));
            }
            for row in 0..h {
                for col in 0..w {
                    let i = (row * w + col) * 4;
                    next[(tile.y + row) * WIDTH + tile.x + col] =
                        u32::from_le_bytes(raw[i..i + 4].try_into().unwrap());
                }
            }
        }
        if self.sequence == 0 && seen.len() != WIDTH.div_ceil(TILE) * HEIGHT.div_ceil(TILE) {
            return Err(invalid("incomplete initial frame"));
        }
        self.pixels = next;
        self.sequence = sequence;
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
