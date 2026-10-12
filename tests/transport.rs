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
            tiles: vec![],
        },
        Message::Frame {
            sequence: 2,
            tiles: vec![Tile {
                x: WIDTH,
                y: 0,
                data: vec![],
            }],
        },
        Message::Frame {
            sequence: 2,
            tiles: vec![Tile {
                x: 0,
                y: 0,
                data: vec![255],
            }],
        },
        Message::Frame {
            sequence: 2,
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
