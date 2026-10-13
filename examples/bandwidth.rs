//! Compare the same compressed tiles with the exact v3 JSON and v4 binary envelopes.
#[path = "support/legacy_json.rs"]
mod legacy_json;
use std::{error::Error, time::Instant};
use waytransfer::*;
fn report(label: &str, frame: &Message, raw: usize) -> Result<(), Box<dyn Error>> {
    let json = legacy_json::frame_bytes(frame);
    let mut bytes = Vec::new();
    let start = Instant::now();
    let binary = write_message(&mut bytes, frame)?;
    let encode = start.elapsed();
    let mut decoder = Decoder::default();
    // Reconstruction is separately tested for deltas; this measurement compares envelopes.
    if matches!(frame, Message::Frame { reset: true, .. }) {
        decoder.apply(read_message(std::io::Cursor::new(bytes))?)?;
    }
    println!("{label:22} raw={raw:8} v3_json={json:8} v4_binary={binary:8} saved={:.1}% binary_encode={encode:?}",(1.0-binary as f64/json as f64)*100.0);
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    for size in [Dimensions::default(), Dimensions::new(1920, 1080)?] {
        let mut encoder = Encoder::default();
        let pixels = demo_frame_sized(size, 320, 200, false)?;
        report(
            &format!("{}x{} initial", size.width, size.height),
            &encoder.encode_sized(size, &pixels)?.unwrap(),
            size.pixel_count() * 4,
        )?;
        let pixels = demo_frame_sized(size, 321, 200, false)?;
        report(
            &format!("{}x{} 1px motion", size.width, size.height),
            &encoder.encode_sized(size, &pixels)?.unwrap(),
            size.pixel_count() * 4,
        )?;
        assert!(encoder.encode_sized(size, &pixels)?.is_none());
        println!("Idle unchanged frame: 0 bytes");
    }
    let size = Dimensions::new(1920, 1080)?;
    let mut seed = 1u32;
    let noise: Vec<_> = (0..size.pixel_count())
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        })
        .collect();
    report(
        "1920x1080 noise",
        &Encoder::default().encode_sized(size, &noise)?.unwrap(),
        size.pixel_count() * 4,
    )?;
    Ok(())
}
