//! Protocol v4: length-prefixed binary messages; see PROTOCOL.md for byte layout.
use crate::{invalid, Dimensions, Message, Tile, MAX_DIMENSION, MAX_PACKET, TILE};
use std::io::{self, Read, Write};

fn tile_limit(size: Dimensions, x: usize, y: usize) -> io::Result<usize> {
    if x >= size.width as usize
        || y >= size.height as usize
        || !x.is_multiple_of(TILE)
        || !y.is_multiple_of(TILE)
    {
        return Err(invalid("invalid tile coordinates"));
    }
    let width = TILE.min(size.width as usize - x);
    let height = TILE.min(size.height as usize - y);
    Ok(lz4_flex::block::get_maximum_output_size(width * height * 4))
}
fn tile_count(size: Dimensions, count: usize) -> io::Result<()> {
    if count == 0
        || count > (size.width as usize).div_ceil(TILE) * (size.height as usize).div_ceil(TILE)
    {
        return Err(invalid("invalid tile count"));
    }
    Ok(())
}
fn pointer_bounds(x: u16, y: u16) -> io::Result<()> {
    if x as usize >= MAX_DIMENSION || y as usize >= MAX_DIMENSION {
        return Err(invalid("pointer outside protocol limits"));
    }
    Ok(())
}
fn put_size(out: &mut Vec<u8>, size: Dimensions) -> io::Result<()> {
    size.validate()?;
    out.extend(size.width.to_be_bytes());
    out.extend(size.height.to_be_bytes());
    Ok(())
}
/// Validate and encode completely before emitting any bytes to the stream.
pub fn write_message(mut writer: impl Write, message: &Message) -> io::Result<usize> {
    let mut out = Vec::new();
    match message {
        Message::Hello { version } => {
            out.push(1);
            out.extend(version.to_be_bytes());
        }
        Message::Frame {
            sequence,
            size,
            reset,
            tiles,
        } => {
            size.validate()?;
            tile_count(*size, tiles.len())?;
            if *sequence == 0 {
                return Err(invalid("zero frame sequence"));
            }
            out.push(2);
            out.extend(sequence.to_be_bytes());
            put_size(&mut out, *size)?;
            out.push(u8::from(*reset));
            out.extend((tiles.len() as u16).to_be_bytes());
            for tile in tiles {
                let limit = tile_limit(*size, tile.x, tile.y)?;
                if tile.data.is_empty() || tile.data.len() > limit {
                    return Err(invalid("invalid compressed tile length"));
                }
                out.extend((tile.x as u16).to_be_bytes());
                out.extend((tile.y as u16).to_be_bytes());
                out.extend((tile.data.len() as u16).to_be_bytes());
                out.extend(&tile.data);
            }
        }
        Message::Pointer { x, y, pressed } => {
            pointer_bounds(*x, *y)?;
            out.push(3);
            out.extend(x.to_be_bytes());
            out.extend(y.to_be_bytes());
            out.push(u8::from(*pressed));
        }
        Message::Resize { size } => {
            out.push(4);
            put_size(&mut out, *size)?;
        }
        Message::Keys { held } => {
            if held.len() > 128 || held.iter().any(|key| !(1..=127).contains(key)) {
                return Err(invalid("invalid key snapshot"));
            }
            out.push(5);
            out.push(held.len() as u8);
            out.extend(held.iter().map(|key| *key as u8));
        }
        Message::Buttons { right, middle } => {
            out.extend([6, u8::from(*right), u8::from(*middle)]);
        }
        Message::Scroll {
            horizontal,
            vertical,
        } => {
            out.push(7);
            out.extend(horizontal.to_be_bytes());
            out.extend(vertical.to_be_bytes());
        }
        Message::Focus { active } => {
            out.extend([8, u8::from(*active)]);
        }
    }
    if out.len() > MAX_PACKET {
        return Err(invalid("packet too large"));
    }
    writer.write_all(&(out.len() as u32).to_be_bytes())?;
    writer.write_all(&out)?;
    Ok(out.len() + 4)
}
struct Cursor<'a> {
    bytes: &'a [u8],
}
impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if count > self.bytes.len() {
            return Err(invalid("truncated message body"));
        }
        let (value, tail) = self.bytes.split_at(count);
        self.bytes = tail;
        Ok(value)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i16(&mut self) -> io::Result<i16> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn boolean(&mut self) -> io::Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("invalid boolean")),
        }
    }
    fn size(&mut self) -> io::Result<Dimensions> {
        Dimensions::new(self.u16()? as usize, self.u16()? as usize)
    }
}
pub fn read_message(reader: impl Read) -> io::Result<Message> {
    read_message_limit(reader, MAX_PACKET)
}
/// Reject the length before allocating. Control channels can impose a smaller limit.
pub fn read_message_limit(mut reader: impl Read, limit: usize) -> io::Result<Message> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > limit.min(MAX_PACKET) {
        return Err(invalid("invalid packet length"));
    }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    let mut cursor = Cursor { bytes: &bytes };
    let message = match cursor.u8()? {
        1 => Message::Hello {
            version: cursor.u16()?,
        },
        2 => {
            let sequence = cursor.u64()?;
            if sequence == 0 {
                return Err(invalid("zero frame sequence"));
            }
            let size = cursor.size()?;
            let reset = cursor.boolean()?;
            let count = cursor.u16()? as usize;
            tile_count(size, count)?;
            // Six bytes per tile header plus at least one data byte; validate before reserve.
            if count > cursor.bytes.len() / 7 {
                return Err(invalid("truncated tile list"));
            }
            let mut tiles = Vec::with_capacity(count);
            for _ in 0..count {
                let x = cursor.u16()? as usize;
                let y = cursor.u16()? as usize;
                let len = cursor.u16()? as usize;
                let limit = tile_limit(size, x, y)?;
                if len == 0 || len > limit {
                    return Err(invalid("invalid compressed tile length"));
                }
                tiles.push(Tile {
                    x,
                    y,
                    data: cursor.take(len)?.to_vec(),
                });
            }
            Message::Frame {
                sequence,
                size,
                reset,
                tiles,
            }
        }
        3 => {
            let x = cursor.u16()?;
            let y = cursor.u16()?;
            pointer_bounds(x, y)?;
            Message::Pointer {
                x,
                y,
                pressed: cursor.boolean()?,
            }
        }
        4 => Message::Resize {
            size: cursor.size()?,
        },
        5 => {
            let len = cursor.u8()? as usize;
            if len > 128 {
                return Err(invalid("invalid key count"));
            }
            let held = cursor.take(len)?;
            if held.iter().any(|key| !(1..=127).contains(key)) {
                return Err(invalid("invalid key code"));
            }
            Message::Keys {
                held: held.iter().map(|key| u16::from(*key)).collect(),
            }
        }
        6 => Message::Buttons {
            right: cursor.boolean()?,
            middle: cursor.boolean()?,
        },
        7 => Message::Scroll {
            horizontal: cursor.i16()?,
            vertical: cursor.i16()?,
        },
        8 => Message::Focus {
            active: cursor.boolean()?,
        },
        b'{' => {
            return Err(invalid(
                "older JSON protocol detected; update both machines",
            ))
        }
        _ => return Err(invalid("unknown message tag")),
    };
    if !cursor.bytes.is_empty() {
        return Err(invalid("trailing bytes"));
    }
    Ok(message)
}
