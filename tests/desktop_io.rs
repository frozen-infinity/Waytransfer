#![cfg(all(target_os = "linux", feature = "desktop"))]
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    thread,
    time::{Duration, Instant},
};
use wayland_client::{backend::WaylandError, Connection};
use waytransfer::desktop::{pump, DesktopState};

fn callback_connection() -> (Connection, UnixStream, u32) {
    let (client, mut peer) = UnixStream::pair().unwrap();
    let server = thread::spawn(move || {
        let mut request = [0u8; 12];
        peer.read_exact(&mut request).unwrap();
        assert_eq!(u32::from_ne_bytes(request[0..4].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_ne_bytes(request[4..8].try_into().unwrap()),
            12 << 16
        );
        let id = u32::from_ne_bytes(request[8..12].try_into().unwrap());
        // wl_callback.done completes roundtrip; delete_id deliberately arrives later.
        for value in [id, 12 << 16, 1] {
            peer.write_all(&value.to_ne_bytes()).unwrap();
        }
        (peer, id)
    });
    let connection = Connection::from_socket(client).unwrap();
    connection.roundtrip().unwrap();
    let (peer, id) = server.join().unwrap();
    (connection, peer, id)
}
fn delete_only(fragmented: bool) {
    let (connection, mut peer, id) = callback_connection();
    let mut queue = connection.new_event_queue();
    let mut state = DesktopState::default();
    let packet: Vec<_> = [1u32, (12 << 16) | 1, id]
        .into_iter()
        .flat_map(u32::to_ne_bytes)
        .collect();
    if fragmented {
        peer.write_all(&packet[..8]).unwrap();
        pump(
            &connection,
            &mut queue,
            &mut state,
            Duration::from_millis(10),
        )
        .unwrap();
        peer.write_all(&packet[8..]).unwrap();
    } else {
        peer.write_all(&packet).unwrap();
    }
    pump(
        &connection,
        &mut queue,
        &mut state,
        Duration::from_millis(10),
    )
    .unwrap();
    pump(&connection, &mut queue, &mut state, Duration::ZERO).unwrap();
    assert!(connection.backend().last_error().is_none());
    drop(peer);
    assert!(
        pump(
            &connection,
            &mut queue,
            &mut state,
            Duration::from_millis(10)
        )
        .is_err(),
        "disconnect must remain fatal"
    );
}
#[test]
fn internal_delete_id_does_not_stop_capture() {
    delete_only(false);
}
#[test]
fn fragmented_wayland_message_does_not_stop_capture() {
    delete_only(true);
}
#[test]
fn outgoing_backpressure_retries_and_preserves_requests() {
    let (client, mut peer) = UnixStream::pair().unwrap();
    let connection = Connection::from_socket(client).unwrap();
    let mut queue = connection.new_event_queue();
    let mut state = DesktopState::default();
    // Stop producing requests at the first blocked flush, rather than exhausting
    // the backend's own bounded request buffer while the peer is paused.
    let mut sent = 0;
    loop {
        connection.display().get_registry(&queue.handle(), ());
        sent += 1;
        match connection.flush() {
            Ok(()) => assert!(sent < 30_000, "socket never became backpressured"),
            Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => panic!("{error}"),
        }
    }
    pump(&connection, &mut queue, &mut state, Duration::ZERO).unwrap();
    peer.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut received = 0;
    let mut buffer = [0u8; 8192];
    while received < sent * 12 {
        loop {
            match peer.read(&mut buffer) {
                Ok(0) => panic!("connection closed under write pressure"),
                Ok(count) => received += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("{error}"),
            }
        }
        pump(
            &connection,
            &mut queue,
            &mut state,
            Duration::from_millis(1),
        )
        .unwrap();
        assert!(
            Instant::now() < deadline,
            "outgoing requests were lost or never flushed"
        );
    }
    assert_eq!(received, sent * 12);
}
