use std::io::Cursor;
use waytransfer::*;
fn packet(body: &[u8]) -> Vec<u8> {
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend(body);
    bytes
}
#[test]
fn fixed_control_fixtures_pin_tags_endianness_and_booleans() {
    let cases = [
        (Message::Hello { version: 4 }, vec![1, 0, 4]),
        (
            Message::Pointer {
                x: 10,
                y: 20,
                pressed: true,
            },
            vec![3, 0, 10, 0, 20, 1],
        ),
        (
            Message::Resize {
                size: Dimensions::new(1024, 768).unwrap(),
            },
            vec![4, 4, 0, 3, 0],
        ),
        (Message::Keys { held: vec![42, 30] }, vec![5, 2, 42, 30]),
        (Message::Keys { held: vec![] }, vec![5, 0]),
        (
            Message::Buttons {
                right: true,
                middle: false,
            },
            vec![6, 1, 0],
        ),
        (
            Message::Scroll {
                horizontal: -15,
                vertical: 300,
            },
            vec![7, 255, 241, 1, 44],
        ),
        (Message::Focus { active: false }, vec![8, 0]),
    ];
    for (message, body) in cases {
        let expected = packet(&body);
        let mut bytes = Vec::new();
        assert_eq!(write_message(&mut bytes, &message).unwrap(), expected.len());
        assert_eq!(bytes, expected);
        assert_eq!(read_message(Cursor::new(expected)).unwrap(), message);
    }
}
#[test]
fn independent_one_pixel_frame_fixture_reconstructs_exactly() {
    // Header: frame, sequence=1, size=1x1, reset=1, count=1.
    // Tile: x=0,y=0,len=5; LZ4 literal-only block for LE 0x00112233.
    let body = [
        2, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 1, 1, 0, 1, 0, 0, 0, 0, 0, 5, 0x40, 0x33, 0x22, 0x11, 0,
    ];
    let message = read_message(Cursor::new(packet(&body))).unwrap();
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message).unwrap();
    assert_eq!(bytes, packet(&body));
    let mut decoder = Decoder::default();
    decoder.apply(message).unwrap();
    assert_eq!(decoder.size, Dimensions::new(1, 1).unwrap());
    assert_eq!(decoder.pixels, vec![0x00112233]);
    for end in 0..body.len() {
        assert!(read_message(Cursor::new(packet(&body[..end]))).is_err());
    }
}
#[test]
fn concatenation_and_fragmented_reads_preserve_packet_boundaries() {
    struct OneByte(Cursor<Vec<u8>>);
    impl std::io::Read for OneByte {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = out.len().min(1);
            std::io::Read::read(&mut self.0, &mut out[..n])
        }
    }
    let mut bytes = Vec::new();
    write_message(&mut bytes, &Message::Focus { active: true }).unwrap();
    write_message(&mut bytes, &Message::Keys { held: vec![30] }).unwrap();
    let mut reader = OneByte(Cursor::new(bytes));
    assert_eq!(
        read_message(&mut reader).unwrap(),
        Message::Focus { active: true }
    );
    assert_eq!(
        read_message(&mut reader).unwrap(),
        Message::Keys { held: vec![30] }
    );
    assert!(read_message(reader).is_err());
}
#[test]
fn malformed_control_fields_are_rejected() {
    for body in [
        vec![0],
        vec![255],
        vec![8, 2],
        vec![8, 1, 0],
        vec![6, 0, 3],
        vec![3, 255, 255, 0, 1, 0],
        vec![4, 0, 0, 0, 1],
        vec![5, 129],
        vec![5, 1, 0],
        vec![5, 1, 128],
        vec![7, 0],
    ] {
        assert!(
            read_message(Cursor::new(packet(&body))).is_err(),
            "body: {body:?}"
        );
    }
    assert!(read_message_limit(Cursor::new(packet(&[8, 1])), 1).is_err());
}
#[test]
fn malformed_tile_headers_are_rejected_without_large_reserves() {
    let base = [
        2, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 1, 1, 0, 1, 0, 0, 0, 0, 0, 5, 0x40, 0x33, 0x22, 0x11, 0,
    ];
    for (index, value) in [
        (8, 0),
        (13, 2),
        (14, 255),
        (15, 0),
        (17, 1),
        (21, 0),
        (20, 255),
    ] {
        let mut body = base.to_vec();
        body[index] = value;
        assert!(
            read_message(Cursor::new(packet(&body))).is_err(),
            "mutated byte {index}"
        );
    }
    let mut body = base.to_vec();
    body.push(0);
    assert!(read_message(Cursor::new(packet(&body))).is_err());
}
#[test]
fn invalid_outgoing_message_writes_nothing() {
    for message in [
        Message::Keys { held: vec![0] },
        Message::Resize {
            size: Dimensions {
                width: 0,
                height: 1,
            },
        },
        Message::Frame {
            sequence: 1,
            size: Dimensions::default(),
            reset: false,
            tiles: vec![Tile {
                x: usize::MAX,
                y: 0,
                data: vec![1],
            }],
        },
    ] {
        let mut bytes = Vec::new();
        assert!(write_message(&mut bytes, &message).is_err());
        assert!(bytes.is_empty());
    }
}
#[test]
fn deterministic_malformed_packet_corpus_does_not_panic() {
    let mut seed = 0x12345678u32;
    for len in 0..256 {
        for _ in 0..32 {
            let mut body = vec![0; len];
            for b in &mut body {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *b = seed as u8;
            }
            let _ = read_message(Cursor::new(packet(&body)));
        }
    }
}
