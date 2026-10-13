use std::io::Cursor;
use waytransfer::*;
#[test]
fn lossless_edges_idle_and_small_delta() {
    let mut enc = Encoder::default();
    let mut dec = Decoder::default();
    let mut pixels: Vec<u32> = (0..WIDTH * HEIGHT)
        .map(|i| (i as u32).wrapping_mul(7919))
        .collect();
    dec.apply(enc.encode(&pixels).unwrap().unwrap()).unwrap();
    assert_eq!(dec.pixels, pixels);
    assert!(enc.encode(&pixels).unwrap().is_none());
    pixels[WIDTH * HEIGHT - 1] ^= 0xffffff;
    let update = enc.encode(&pixels).unwrap().unwrap();
    let Message::Frame { ref tiles, .. } = update else {
        panic!()
    };
    assert_eq!(tiles.len(), 1);
    dec.apply(update).unwrap();
    assert_eq!(dec.pixels, pixels);
}
#[test]
fn demo_bandwidth() {
    let mut enc = Encoder::default();
    let mut bytes = Vec::new();
    write_message(
        &mut bytes,
        &enc.encode(&demo_frame(320, 200, false)).unwrap().unwrap(),
    )
    .unwrap();
    assert!(bytes.len() < WIDTH * HEIGHT * 4 / 5);
    bytes.clear();
    write_message(
        &mut bytes,
        &enc.encode(&demo_frame(321, 200, false)).unwrap().unwrap(),
    )
    .unwrap();
    assert!(bytes.len() < 10000);
}
#[test]
fn framing_and_truncation() {
    let message = Message::Pointer {
        x: 10,
        y: 20,
        pressed: true,
    };
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message).unwrap();
    assert_eq!(read_message(Cursor::new(&bytes)).unwrap(), message);
    for end in 0..bytes.len() {
        assert!(read_message(Cursor::new(&bytes[..end])).is_err());
    }
    assert!(read_message(Cursor::new(((MAX_PACKET + 1) as u32).to_be_bytes())).is_err());
    assert!(read_message(Cursor::new([0, 0, 0, 0])).is_err());
}
#[test]
fn rejected_updates_are_atomic() {
    let mut enc = Encoder::default();
    let mut dec = Decoder::default();
    dec.apply(enc.encode(&demo_frame(20, 20, false)).unwrap().unwrap())
        .unwrap();
    let before = dec.pixels.clone();
    for message in [
        Message::Frame {
            sequence: 3,
            size: Dimensions::default(),
            reset: true,
            tiles: vec![],
        },
        Message::Frame {
            sequence: 2,
            size: Dimensions::default(),
            reset: true,
            tiles: vec![Tile {
                x: WIDTH,
                y: 0,
                data: vec![],
            }],
        },
        Message::Frame {
            sequence: 2,
            size: Dimensions::default(),
            reset: true,
            tiles: vec![Tile {
                x: 0,
                y: 0,
                data: vec![255],
            }],
        },
        Message::Frame {
            sequence: 2,
            size: Dimensions::default(),
            reset: true,
            tiles: vec![Tile {
                x: 0,
                y: 0,
                data: lz4_flex::block::compress(&[0; 4]),
            }],
        },
    ] {
        assert!(dec.apply(message).is_err());
        assert_eq!(dec.pixels, before);
    }
    dec.apply(enc.encode(&demo_frame(30, 30, false)).unwrap().unwrap())
        .unwrap();
}
#[test]
fn first_frame_must_be_complete() {
    assert!(Decoder::default()
        .apply(Message::Frame {
            sequence: 1,
            size: Dimensions::default(),
            reset: true,
            tiles: vec![Tile {
                x: 0,
                y: 0,
                data: lz4_flex::block::compress(&vec![0; TILE * TILE * 4])
            }]
        })
        .is_err());
}
#[test]
fn tcp_round_trip_and_input() {
    use std::net::{TcpListener, TcpStream};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let sender = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        handshake(&mut stream).unwrap();
        let mut enc = Encoder::default();
        write_message(
            &mut stream,
            &enc.encode(&demo_frame(20, 20, false)).unwrap().unwrap(),
        )
        .unwrap();
        assert_eq!(
            read_message(&mut stream).unwrap(),
            Message::Pointer {
                x: 40,
                y: 40,
                pressed: true
            }
        );
        write_message(
            &mut stream,
            &enc.encode(&demo_frame(40, 40, true)).unwrap().unwrap(),
        )
        .unwrap();
    });
    let mut stream = TcpStream::connect(addr).unwrap();
    handshake(&mut stream).unwrap();
    let mut dec = Decoder::default();
    dec.apply(read_message(&mut stream).unwrap()).unwrap();
    write_message(
        &mut stream,
        &Message::Pointer {
            x: 40,
            y: 40,
            pressed: true,
        },
    )
    .unwrap();
    dec.apply(read_message(&mut stream).unwrap()).unwrap();
    assert_eq!(dec.pixels, demo_frame(40, 40, true));
    sender.join().unwrap();
}

#[test]
fn incompatible_handshake_is_rejected() {
    use std::net::{TcpListener, TcpStream};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        write_message(
            &mut stream,
            &Message::Hello {
                version: VERSION + 1,
            },
        )
        .unwrap();
        assert_eq!(
            read_message(&mut stream).unwrap(),
            Message::Hello { version: VERSION }
        );
    });
    assert!(handshake(&mut TcpStream::connect(addr).unwrap()).is_err());
    peer.join().unwrap();
}

#[test]
fn resize_resets_tiles_and_same_area_shape_change_is_not_a_delta() {
    let mut encoder = Encoder::default();
    let mut decoder = Decoder::default();
    for size in [
        Dimensions::new(97, 65).unwrap(),
        Dimensions::new(65, 97).unwrap(),
        Dimensions::new(640, 400).unwrap(),
        Dimensions::new(17, 19).unwrap(),
    ] {
        let pixels: Vec<_> = (0..size.pixel_count()).map(|i| i as u32 * 17).collect();
        let frame = encoder.encode_sized(size, &pixels).unwrap().unwrap();
        assert!(matches!(frame, Message::Frame { reset: true, .. }));
        decoder.apply(frame).unwrap();
        assert_eq!(decoder.size, size);
        assert_eq!(decoder.pixels, pixels);
        assert!(encoder.encode_sized(size, &pixels).unwrap().is_none());
        let mut changed = pixels.clone();
        *changed.last_mut().unwrap() ^= 0xff;
        let delta = encoder.encode_sized(size, &changed).unwrap().unwrap();
        assert!(matches!(delta, Message::Frame { reset: false, .. }));
        decoder.apply(delta).unwrap();
        assert_eq!(decoder.pixels, changed);
    }
}
#[test]
fn invalid_resize_frame_leaves_old_dimensions_and_pixels_intact() {
    let mut encoder = Encoder::default();
    let mut decoder = Decoder::default();
    decoder
        .apply(encoder.encode(&demo_frame(40, 40, false)).unwrap().unwrap())
        .unwrap();
    let original = decoder.pixels.clone();
    for (size, reset) in [
        (
            Dimensions {
                width: 0,
                height: 400,
            },
            true,
        ),
        (
            Dimensions {
                width: 65535,
                height: 1,
            },
            true,
        ),
        (Dimensions::new(700, 500).unwrap(), false),
        (Dimensions::new(700, 500).unwrap(), true),
    ] {
        assert!(decoder
            .apply(Message::Frame {
                sequence: 2,
                size,
                reset,
                tiles: vec![Tile {
                    x: 0,
                    y: 0,
                    data: lz4_flex::block::compress(&vec![0; TILE * TILE * 4])
                }]
            })
            .is_err());
        assert_eq!(decoder.size, Dimensions::default());
        assert_eq!(decoder.pixels, original);
    }
    decoder
        .apply(encoder.encode(&demo_frame(41, 40, false)).unwrap().unwrap())
        .unwrap();
}
#[test]
fn dimension_bounds_and_fitting() {
    assert!("1920x1080".parse::<Dimensions>().is_ok());
    for text in ["0x400", "2049x1", "2048x2048", "huge", "-1x5"] {
        assert!(text.parse::<Dimensions>().is_err());
    }
    let fitted = Dimensions::fit(4096, 2160);
    assert!(fitted.validate().is_ok());
    assert!(fitted.pixel_count() <= MAX_PIXELS);
    assert!((f64::from(fitted.width) / f64::from(fitted.height) - 4096.0 / 2160.0).abs() < 0.01);
}
#[test]
fn incompressible_full_hd_frame_fits_packet_limit() {
    let size = Dimensions::new(1920, 1080).unwrap();
    let mut state = 1u32;
    let pixels: Vec<_> = (0..size.pixel_count())
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        })
        .collect();
    let frame = Encoder::default()
        .encode_sized(size, &pixels)
        .unwrap()
        .unwrap();
    let mut bytes = Vec::new();
    write_message(&mut bytes, &frame).unwrap();
    assert!(bytes.len() <= MAX_PACKET + 4);
    let mut decoder = Decoder::default();
    decoder
        .apply(read_message(Cursor::new(bytes)).unwrap())
        .unwrap();
    assert_eq!(decoder.pixels, pixels);
}
