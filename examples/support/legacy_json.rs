//! Exact compact v3 JSON layout for frame-size comparison only; never used on the network.
use std::fmt::Write;
use waytransfer::Message;
pub fn frame_bytes(message: &Message) -> usize {
    let Message::Frame {
        sequence,
        size,
        reset,
        tiles,
    } = message
    else {
        panic!("expected frame")
    };
    let mut json=format!("{{\"Frame\":{{\"sequence\":{sequence},\"size\":{{\"width\":{},\"height\":{}}},\"reset\":{reset},\"tiles\":[",size.width,size.height);
    for (i, tile) in tiles.iter().enumerate() {
        if i != 0 {
            json.push(',');
        }
        write!(json, "{{\"x\":{},\"y\":{},\"data\":[", tile.x, tile.y).unwrap();
        for (i, byte) in tile.data.iter().enumerate() {
            if i != 0 {
                json.push(',');
            }
            write!(json, "{byte}").unwrap();
        }
        json.push_str("]}");
    }
    json.push_str("]}}");
    json.len() + 4
}
