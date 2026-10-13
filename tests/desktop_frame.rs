use waytransfer::{desktop_frame::*, Dimensions};
#[test]
fn padded_inverted_formats_and_scaled_capture() {
    let bytes: Vec<_> = [0x00ff0000u32, 0x0000ff00, 0, 0x000000ff, 0x00112233, 0]
        .into_iter()
        .flat_map(u32::to_ne_bytes)
        .collect();
    let mut layout = CaptureLayout {
        width: 2,
        height: 2,
        stride: 12,
        format: PixelFormat::Rgb,
        inverted: false,
    };
    let (_, pixels) = layout
        .convert(&bytes, Dimensions::new(2, 2).unwrap())
        .unwrap();
    assert_eq!(pixels, vec![0xff0000, 0x00ff00, 0xff, 0x112233]);
    layout.inverted = true;
    assert_eq!(
        layout
            .convert(&bytes, Dimensions::new(2, 2).unwrap())
            .unwrap()
            .1,
        vec![0xff, 0x112233, 0xff0000, 0xff00]
    );
    layout.format = PixelFormat::Bgr;
    assert_eq!(
        layout
            .convert(&bytes, Dimensions::new(1, 1).unwrap())
            .unwrap()
            .1,
        vec![0xff0000]
    );
    assert!(layout
        .convert(&bytes[..bytes.len() - 1], Dimensions::default())
        .is_err());
    layout.width = 8193;
    assert!(layout.byte_len().is_err());
}
#[test]
fn ten_bit_capture_and_aspect_ratio_are_preserved() {
    let layout = CaptureLayout {
        width: 1,
        height: 1,
        stride: 4,
        format: PixelFormat::Rgb10,
        inverted: false,
    };
    assert_eq!(
        layout
            .convert(&(1023u32 << 20).to_ne_bytes(), Dimensions::default())
            .unwrap()
            .1,
        vec![0xff0000]
    );
    let layout = CaptureLayout {
        width: 4,
        height: 2,
        stride: 16,
        format: PixelFormat::Rgb,
        inverted: false,
    };
    assert_eq!(
        layout
            .convert(&[0; 32], Dimensions::new(2, 2).unwrap())
            .unwrap()
            .0,
        Dimensions::new(2, 1).unwrap()
    );
}
