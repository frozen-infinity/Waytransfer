use std::{net::TcpStream, sync::Arc, time::Duration};
use waytransfer::{client_scene::ClientScene, remote::*, *};
fn snapshot(id: u32, color: u32) -> WindowSnapshot {
    let size = Dimensions::new(97, 65).unwrap();
    WindowSnapshot {
        id,
        parent: if id == 2 { Some(1) } else { None },
        title: format!("Application {id}"),
        size,
        pixels: Arc::new(vec![color; size.pixel_count()]),
    }
}
#[test]
fn independent_windows_lifecycle_input_and_reconnect() {
    let server = RemoteServer::bind_multi("127.0.0.1:0").unwrap();
    server
        .publish_windows(vec![snapshot(1, 0x112233), snapshot(2, 0x445566)])
        .unwrap();
    let mut viewer = TcpStream::connect(server.local_addr()).unwrap();
    handshake(&mut viewer).unwrap();
    viewer
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut scene = ClientScene::default();
    while scene.windows.len() != 2 || scene.windows.values().any(|window| !window.ready) {
        scene.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    assert_eq!(scene.windows[&1].decoder.pixels[0], 0x112233);
    assert_eq!(scene.windows[&2].decoder.pixels[0], 0x445566);
    assert_eq!(scene.windows[&2].parent, Some(1));
    let input = Message::ForWindow {
        id: 2,
        message: Box::new(Message::Keys { held: vec![30] }),
    };
    write_message(&mut viewer, &input).unwrap();
    assert!(
        matches!(server.events.recv_timeout(Duration::from_secs(2)).unwrap(),RemoteEvent::Input(message) if message==input)
    );
    scene.close_locally(1);
    server.publish_windows(vec![snapshot(2, 0x778899)]).unwrap();
    while scene.windows[&2].decoder.pixels[0] != 0x778899 || scene.closing.contains(&1) {
        scene.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    assert!(!scene.windows.contains_key(&1));
    drop(viewer);
    server.events.recv_timeout(Duration::from_secs(2)).unwrap();
    let mut viewer = TcpStream::connect(server.local_addr()).unwrap();
    handshake(&mut viewer).unwrap();
    viewer
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut scene = ClientScene::default();
    while scene.windows.get(&2).is_none_or(|window| !window.ready) {
        scene.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    assert_eq!(scene.windows.len(), 1);
    assert_eq!(scene.windows[&2].decoder.pixels[0], 0x778899);
}
#[test]
fn scene_and_envelope_limits() {
    let mut scene = ClientScene::default();
    assert!(scene
        .apply(Message::ForWindow {
            id: 1,
            message: Box::new(Message::Focus { active: true })
        })
        .is_err());
    for id in 1..=MAX_WINDOWS as u32 {
        scene
            .apply(Message::WindowInfo {
                id,
                parent: None,
                size: Dimensions::new(1, 1).unwrap(),
                title: "Window".into(),
            })
            .unwrap();
    }
    assert!(scene
        .apply(Message::WindowInfo {
            id: 99,
            parent: None,
            size: Dimensions::default(),
            title: "Overflow".into()
        })
        .is_err());
    let nested = Message::ForWindow {
        id: 1,
        message: Box::new(Message::ForWindow {
            id: 2,
            message: Box::new(Message::Focus { active: true }),
        }),
    };
    assert!(write_message(Vec::new(), &nested).is_err());
    assert_eq!(window_title("hello\0world\n"), "helloworld");
    assert!(window_title(&"é".repeat(400)).len() <= MAX_TITLE_BYTES);
}

#[test]
fn cyclic_parents_and_mixed_streams_are_rejected() {
    let metadata = |id, parent| Message::WindowInfo {
        id,
        parent,
        size: Dimensions::new(1, 1).unwrap(),
        title: "Window".into(),
    };
    let mut scene = ClientScene::default();
    scene.apply(metadata(1, Some(2))).unwrap();
    assert!(scene.apply(metadata(2, Some(1))).is_err());
    assert_eq!(scene.windows.len(), 1);
    let mut scene = ClientScene::default();
    let mut encoder = Encoder::default();
    scene
        .apply(encoder.encode(&demo_frame(10, 10, false)).unwrap().unwrap())
        .unwrap();
    assert!(scene.apply(metadata(2, None)).is_err());
}
