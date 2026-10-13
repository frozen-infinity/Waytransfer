use std::{net::TcpStream, time::Duration};
use waytransfer::{
    remote::{RemoteEvent, RemoteServer},
    *,
};
#[test]
fn reconnect_gets_complete_latest_frame_and_input_round_trip() {
    let server = RemoteServer::bind("127.0.0.1:0").unwrap();
    let first = demo_frame(30, 30, false);
    server.publish(first.clone()).unwrap();
    let mut stream = TcpStream::connect(server.local_addr()).unwrap();
    handshake(&mut stream).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut decoder = Decoder::default();
    decoder.apply(read_message(&mut stream).unwrap()).unwrap();
    assert_eq!(decoder.pixels, first);
    let message = Message::Keys { held: vec![42, 30] };
    write_message(&mut stream, &message).unwrap();
    assert!(
        matches!(server.events.recv_timeout(Duration::from_secs(2)).unwrap(),RemoteEvent::Input(input) if input == message)
    );
    let final_frame = demo_frame(300, 300, true);
    for x in 31..60 {
        server.publish(demo_frame(x, 30, false)).unwrap();
    }
    server.publish(final_frame.clone()).unwrap();
    while decoder.pixels != final_frame {
        decoder.apply(read_message(&mut stream).unwrap()).unwrap();
    }
    drop(stream);
    assert!(matches!(
        server.events.recv_timeout(Duration::from_secs(2)).unwrap(),
        RemoteEvent::Disconnected
    ));
    let mut reconnected = TcpStream::connect(server.local_addr()).unwrap();
    handshake(&mut reconnected).unwrap();
    reconnected
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut decoder = Decoder::default();
    decoder
        .apply(read_message(&mut reconnected).unwrap())
        .unwrap();
    assert_eq!(decoder.pixels, final_frame);
    // Drop must interrupt the reader and wake the writer even when a live viewer is idle.
    drop(server);
    assert!(read_message(&mut reconnected).is_err());
}

#[test]
fn oversized_input_is_rejected_before_allocation() {
    use std::io::Write;
    let server = RemoteServer::bind("127.0.0.1:0").unwrap();
    let mut stream = TcpStream::connect(server.local_addr()).unwrap();
    handshake(&mut stream).unwrap();
    // A length prefix alone is enough to reject it: no need to send/allocate a body.
    stream.write_all(&4097u32.to_be_bytes()).unwrap();
    assert!(matches!(
        server.events.recv_timeout(Duration::from_secs(2)).unwrap(),
        RemoteEvent::Disconnected
    ));
}
