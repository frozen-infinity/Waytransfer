#[path = "../examples/support/legacy_json.rs"]
mod legacy_json;
use waytransfer::*;
#[test]
fn binary_envelope_reduces_the_same_tile_payload_without_loss() {
    let mut encoder = Encoder::default();
    let mut decoder = Decoder::default();
    for x in [320, 321] {
        let pixels = demo_frame(x, 200, false);
        let frame = encoder.encode(&pixels).unwrap().unwrap();
        let json = legacy_json::frame_bytes(&frame);
        let mut bytes = Vec::new();
        write_message(&mut bytes, &frame).unwrap();
        assert!(bytes.len() * 2 < json);
        decoder
            .apply(read_message(std::io::Cursor::new(bytes)).unwrap())
            .unwrap();
        assert_eq!(decoder.pixels, pixels);
    }
}

#[test]
fn v3_reference_envelope_matches_an_independent_json_fixture() {
    let frame = Message::Frame {
        sequence: 1,
        size: Dimensions::new(1, 1).unwrap(),
        reset: true,
        tiles: vec![Tile {
            x: 0,
            y: 0,
            data: vec![64, 51, 34, 17, 0],
        }],
    };
    let json = r#"{"Frame":{"sequence":1,"size":{"width":1,"height":1},"reset":true,"tiles":[{"x":0,"y":0,"data":[64,51,34,17,0]}]}}"#;
    assert_eq!(legacy_json::frame_bytes(&frame), json.len() + 4);
}
