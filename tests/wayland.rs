#![cfg(all(target_os = "linux", feature = "compositor"))]
use std::{
    io::Write,
    net::TcpStream,
    os::{fd::AsFd, unix::net::UnixStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use wayland_client::{
    delegate_noop,
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat,
        wl_shm, wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use waytransfer::{
    compositor::SingleWindow,
    remote::{RemoteEvent, RemoteServer},
    *,
};

#[derive(Default)]
struct App {
    title: String,
    color: u32,
    closed: bool,
    size: Dimensions,
    surface: Option<wl_surface::WlSurface>,
    shm: Option<wl_shm::WlShm>,
    shell: Option<xdg_wm_base::XdgWmBase>,
    xdg: Option<xdg_surface::XdgSurface>,
    configured: bool,
    key_pressed: bool,
    key_seen: bool,
    click: Option<(usize, usize)>,
    pointer: (usize, usize),
    events: Option<mpsc::Sender<&'static str>>,
}
impl App {
    fn notify(&self, event: &'static str) {
        self.events.as_ref().unwrap().send(event).unwrap();
    }
    fn init(&mut self, qh: &QueueHandle<Self>) {
        if self.xdg.is_none() {
            if let (Some(shell), Some(surface)) = (&self.shell, &self.surface) {
                let xdg = shell.get_xdg_surface(surface, qh, ());
                let top = xdg.get_toplevel(qh, ());
                top.set_title(if self.title.is_empty() {
                    "Waytransfer integration fixture".into()
                } else {
                    self.title.clone()
                });
                self.xdg = Some(xdg);
                surface.commit();
            }
        }
    }
    fn pixels(&self) -> Vec<u32> {
        let (width, height) = (self.size.width as usize, self.size.height as usize);
        let mut pixels = vec![
            if self.color == 0 {
                0x102030
            } else {
                self.color
            };
            width * height
        ];
        // Distinct corners detect flipped output and incorrect channel order.
        pixels[0] = 0xff0000;
        pixels[width - 1] = 0x00ff00;
        pixels[width * (height - 1)] = 0x0000ff;
        if self.key_seen {
            pixels[10 * width + 10] = if self.key_pressed { 0xabcdef } else { 0x123456 };
        }
        if let Some((x, y)) = self.click {
            if x < width && y < height {
                pixels[y * width + x] = 0x55aa22;
            }
        }
        pixels
    }
    fn paint(&self, qh: &QueueHandle<Self>) {
        let (width, height) = (self.size.width as usize, self.size.height as usize);
        if !self.configured {
            return;
        }
        if let (Some(shm), Some(surface)) = (&self.shm, &self.surface) {
            // A fresh buffer each commit: never mutate a buffer still owned by the compositor.
            let mut file = tempfile::tempfile().unwrap();
            for pixel in self.pixels() {
                file.write_all(&(pixel | 0xff000000).to_ne_bytes()).unwrap();
            }
            let pool = shm.create_pool(file.as_fd(), (width * height * 4) as i32, qh, ());
            let buffer = pool.create_buffer(
                0,
                width as i32,
                height as i32,
                (width * 4) as i32,
                wl_shm::Format::Argb8888,
                qh,
                (),
            );
            pool.destroy();
            surface.attach(Some(&buffer), 0, 0);
            surface.damage_buffer(0, 0, width as i32, height as i32);
            surface.frame(qh, ());
            surface.commit();
        }
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    let c = registry.bind::<wl_compositor::WlCompositor, _, _>(
                        name,
                        version.min(4),
                        qh,
                        (),
                    );
                    state.surface = Some(c.create_surface(qh, ()));
                    state.init(qh);
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind(name, 1, qh, ()));
                    state.paint(qh);
                }
                "xdg_wm_base" => {
                    state.shell = Some(registry.bind(name, version.min(3), qh, ()));
                    state.init(qh);
                }
                "wl_seat" => {
                    registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(7), qh, ());
                }
                _ => {}
            }
        }
    }
}
impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _: &mut Self,
        shell: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            shell.pong(serial);
        }
    }
}
impl Dispatch<xdg_surface::XdgSurface, ()> for App {
    fn event(
        state: &mut Self,
        xdg: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg.ack_configure(serial);
            state.configured = true;
            state.paint(qh);
        }
    }
}
impl Dispatch<xdg_toplevel::XdgToplevel, ()> for App {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            state.closed = true;
            state.notify("close");
        }
        if let xdg_toplevel::Event::Configure { width, height, .. } = event {
            state.size = Dimensions::new(width as usize, height as usize).unwrap();
            state.notify("configure");
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        _: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, ());
            }
            if capabilities.contains(wl_seat::Capability::Pointer) {
                seat.get_pointer(qh, ());
            }
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { .. } => state.notify("keyboard-enter"),
            wl_keyboard::Event::Key {
                key: 30,
                state: key_state,
                ..
            } => {
                state.key_seen = true;
                state.key_pressed = key_state == WEnum::Value(wl_keyboard::KeyState::Pressed);
                state.notify(if state.key_pressed {
                    "key-press"
                } else {
                    "key-release"
                });
                state.paint(qh);
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            }
            | wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer = (surface_x as usize, surface_y as usize);
            }
            wl_pointer::Event::Button {
                button: 0x110,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } => {
                state.click = Some(state.pointer);
                state.notify("click");
                state.paint(qh);
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_buffer::WlBuffer, ()> for App {
    fn event(
        _: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            buffer.destroy();
        }
    }
}
impl Dispatch<wl_callback::WlCallback, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.notify("frame-done");
    }
}
delegate_noop!(App: ignore wl_compositor::WlCompositor);
delegate_noop!(App: ignore wl_surface::WlSurface);
delegate_noop!(App: ignore wl_shm::WlShm);
delegate_noop!(App: ignore wl_shm_pool::WlShmPool);

#[test]
fn real_wayland_shm_pixels_pointer_keyboard_focus_and_disconnect() {
    let (client_stream, server_stream) = UnixStream::pair().unwrap();
    let remote = RemoteServer::bind("127.0.0.1:0").unwrap();
    let addr = remote.local_addr();
    let running = Arc::new(AtomicBool::new(true));
    let server_running = running.clone();
    // All Wayland/renderer resources stay on this thread; no real desktop session is needed.
    let server = thread::spawn(move || {
        let mut display = SingleWindow::new_display().unwrap();
        let dh = display.handle();
        let mut state = SingleWindow::new(&dh).unwrap();
        state.insert_client(&dh, server_stream).unwrap();
        let (mut renderer, mut image) = SingleWindow::renderer().unwrap();
        while server_running.load(Ordering::Relaxed) {
            display.dispatch_clients(&mut state).unwrap();
            for _ in 0..128 {
                match remote.events.try_recv() {
                    Ok(RemoteEvent::Input(message)) => state.input(message).unwrap(),
                    Ok(RemoteEvent::Disconnected) => state.release_input(),
                    Err(_) => break,
                }
            }
            if let Some(pixels) = state.render(&mut renderer, &mut image).unwrap() {
                remote.publish_sized(state.dimensions(), pixels).unwrap();
            }
            display.flush_clients().unwrap();
            thread::sleep(Duration::from_millis(5));
        }
    });
    struct Stop(Arc<AtomicBool>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Relaxed);
        }
    }
    let guard = Stop(running);
    let (events_tx, events) = mpsc::channel();
    let client = thread::spawn(move || {
        let connection = Connection::from_socket(client_stream).unwrap();
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        let mut app = App {
            events: Some(events_tx),
            ..Default::default()
        };
        while !app.closed && queue.blocking_dispatch(&mut app).is_ok() {}
    });
    let mut viewer = TcpStream::connect(addr).unwrap();
    handshake(&mut viewer).unwrap();
    viewer
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut decoder = Decoder::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while decoder.pixels[0] != 0xff0000 {
        assert!(Instant::now() < deadline);
        decoder.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    let baseline = App::default().pixels();
    assert_eq!(decoder.pixels, baseline);
    write_message(
        &mut viewer,
        &Message::Pointer {
            x: 60,
            y: 80,
            pressed: true,
        },
    )
    .unwrap();
    while decoder.pixels[80 * WIDTH + 60] != 0x55aa22 {
        decoder.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    write_message(&mut viewer, &Message::Keys { held: vec![30] }).unwrap();
    while decoder.pixels[10 * WIDTH + 10] != 0xabcdef {
        decoder.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    write_message(&mut viewer, &Message::Focus { active: false }).unwrap();
    while decoder.pixels[10 * WIDTH + 10] != 0x123456 {
        decoder.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    write_message(&mut viewer, &Message::Focus { active: true }).unwrap();
    write_message(&mut viewer, &Message::Keys { held: vec![30] }).unwrap();
    while decoder.pixels[10 * WIDTH + 10] != 0xabcdef {
        decoder.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    drop(viewer);
    let mut reconnected = TcpStream::connect(addr).unwrap();
    handshake(&mut reconnected).unwrap();
    reconnected
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut after = Decoder::default();
    while after.pixels[10 * WIDTH + 10] != 0x123456 {
        after
            .apply(read_message(&mut reconnected).unwrap())
            .unwrap();
    }
    // Odd sizes exercise partial tiles, Pixman stride and xdg configure/commit.
    for size in [
        Dimensions::new(801, 503).unwrap(),
        Dimensions::new(351, 227).unwrap(),
    ] {
        write_message(&mut reconnected, &Message::Resize { size }).unwrap();
        let expected = App {
            size,
            key_seen: true,
            click: Some((60, 80)),
            ..Default::default()
        }
        .pixels();
        let deadline = Instant::now() + Duration::from_secs(5);
        while after.size != size || after.pixels != expected {
            assert!(
                Instant::now() < deadline,
                "client did not commit the resized buffer"
            );
            after
                .apply(read_message(&mut reconnected).unwrap())
                .unwrap();
        }
    }
    let size = Dimensions::new(801, 503).unwrap();
    write_message(&mut reconnected, &Message::Resize { size }).unwrap();
    let expected = App {
        size,
        key_seen: true,
        click: Some((60, 80)),
        ..Default::default()
    }
    .pixels();
    while after.size != size || after.pixels != expected {
        after
            .apply(read_message(&mut reconnected).unwrap())
            .unwrap();
    }
    // Input beyond the old 640x400 bounds must be routed after enlargement.
    write_message(
        &mut reconnected,
        &Message::Pointer {
            x: 700,
            y: 450,
            pressed: true,
        },
    )
    .unwrap();
    while after.pixels[450 * 801 + 700] != 0x55aa22 {
        after
            .apply(read_message(&mut reconnected).unwrap())
            .unwrap();
    }
    let received: Vec<_> = events.try_iter().collect();
    assert!(received.contains(&"configure"));
    assert!(received.contains(&"keyboard-enter"));
    assert!(received.contains(&"frame-done"));
    assert!(received.contains(&"click"));
    drop(reconnected);
    drop(guard);
    server.join().unwrap();
    client.join().unwrap();
}

#[test]
fn two_real_wayland_windows_keep_pixels_focus_resize_and_close_independent() {
    multiwindow_roundtrip(false);
}
#[cfg(feature = "gpu")]
#[test]
fn egl_renderer_reconstructs_shm_corners_without_flip_or_channel_errors() {
    if std::env::var("WAYTRANSFER_TEST_GPU").as_deref() != Ok("1") {
        return;
    }
    multiwindow_roundtrip(true);
}
fn multiwindow_roundtrip(gpu: bool) {
    use waytransfer::{client_scene::ClientScene, render_engine::RenderEngine};
    let remote = RemoteServer::bind_multi("127.0.0.1:0").unwrap();
    let addr = remote.local_addr();
    let mut peers = Vec::new();
    let mut streams = Vec::new();
    for _ in 0..2 {
        let (peer, stream) = UnixStream::pair().unwrap();
        peers.push(peer);
        streams.push(stream);
    }
    let running = Arc::new(AtomicBool::new(true));
    let server_running = running.clone();
    let (ready_tx, ready) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut display = SingleWindow::new_display().unwrap();
        let dh = display.handle();
        let mut state = SingleWindow::new(&dh).unwrap();
        for stream in streams {
            state.insert_client(&dh, stream).unwrap();
        }
        let mut renderer = if gpu {
            #[cfg(feature = "gpu")]
            {
                let renderer = RenderEngine::gpu(&dh).unwrap();
                state.advertise_gpu(&renderer).unwrap();
                renderer
            }
            #[cfg(not(feature = "gpu"))]
            {
                panic!("GPU feature required")
            }
        } else {
            RenderEngine::software().unwrap()
        };
        ready_tx.send(()).unwrap();
        while server_running.load(Ordering::Relaxed) {
            display.dispatch_clients(&mut state).unwrap();
            for _ in 0..128 {
                match remote.events.try_recv() {
                    Ok(RemoteEvent::Input(message)) => state.input(message).unwrap(),
                    Ok(RemoteEvent::Disconnected) => state.release_input(),
                    Err(_) => break,
                }
            }
            if let Some(windows) = state.render_windows(&mut renderer).unwrap() {
                remote.publish_windows(windows).unwrap();
            }
            display.flush_clients().unwrap();
            thread::sleep(Duration::from_millis(5));
        }
    });
    struct Stop(Arc<AtomicBool>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Relaxed);
        }
    }
    let guard = Stop(running);
    ready
        .recv_timeout(Duration::from_secs(30))
        .expect("renderer startup failed");
    let mut clients = Vec::new();
    let mut receivers = Vec::new();
    for (index, peer) in peers.into_iter().enumerate() {
        let (tx, rx) = mpsc::channel();
        receivers.push(rx);
        clients.push(thread::spawn(move || {
            let connection = Connection::from_socket(peer).unwrap();
            let mut queue = connection.new_event_queue();
            let qh = queue.handle();
            connection.display().get_registry(&qh, ());
            let mut app = App {
                title: format!("App {index}"),
                color: if index == 0 { 0x102030 } else { 0x405060 },
                events: Some(tx),
                ..Default::default()
            };
            while !app.closed && queue.blocking_dispatch(&mut app).is_ok() {}
        }));
    }
    let mut viewer = TcpStream::connect(addr).unwrap();
    handshake(&mut viewer).unwrap();
    viewer
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut scene = ClientScene::default();
    while scene.windows.len() != 2 || scene.windows.values().any(|window| !window.ready) {
        scene.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    let first = *scene
        .windows
        .iter()
        .find(|(_, window)| window.title == "App 0")
        .unwrap()
        .0;
    let second = *scene
        .windows
        .iter()
        .find(|(_, window)| window.title == "App 1")
        .unwrap()
        .0;
    assert_eq!(
        scene.windows[&first].decoder.pixels,
        App::default().pixels()
    );
    assert_eq!(
        scene.windows[&second].decoder.pixels,
        App {
            color: 0x405060,
            ..Default::default()
        }
        .pixels()
    );
    let send = |viewer: &mut TcpStream, id, message| {
        write_message(
            viewer,
            &Message::ForWindow {
                id,
                message: Box::new(message),
            },
        )
        .unwrap()
    };
    send(&mut viewer, second, Message::Focus { active: true });
    send(&mut viewer, second, Message::Keys { held: vec![30] });
    while scene.windows[&second].decoder.pixels[10 * WIDTH + 10] != 0xabcdef {
        scene.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    assert_eq!(
        scene.windows[&first].decoder.pixels[10 * WIDTH + 10],
        0x102030
    );
    send(&mut viewer, first, Message::Focus { active: true });
    send(&mut viewer, first, Message::Keys { held: vec![30] });
    while scene.windows[&first].decoder.pixels[10 * WIDTH + 10] != 0xabcdef
        || scene.windows[&second].decoder.pixels[10 * WIDTH + 10] != 0x123456
    {
        scene.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    let size = Dimensions::new(801, 503).unwrap();
    send(&mut viewer, second, Message::Resize { size });
    while scene.windows[&second].decoder.size != size {
        scene.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    assert_eq!(scene.windows[&first].decoder.size, Dimensions::default());
    send(&mut viewer, first, Message::Close);
    while scene.windows.contains_key(&first) {
        scene.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    assert!(scene.windows.contains_key(&second));
    drop(viewer);
    drop(guard);
    server.join().unwrap();
    for client in clients {
        client.join().unwrap();
    }
    drop(receivers);
}
