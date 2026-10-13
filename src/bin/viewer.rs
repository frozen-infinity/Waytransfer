use minifb::{MouseButton, MouseMode, Window, WindowOptions};
use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    net::{Shutdown, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use waytransfer::*;
struct NativeWindow {
    window: Window,
    resize: viewer_input::ResizeTracker,
    pointer: Option<(u16, u16, bool)>,
    keys: Vec<u16>,
    buttons: (bool, bool),
}
fn scoped(multi: bool, id: u32, message: Message) -> Message {
    if multi {
        Message::ForWindow {
            id,
            message: Box::new(message),
        }
    } else {
        message
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    let headless = std::env::args().any(|arg| arg == "--headless");
    let desktop = std::env::args().any(|arg| arg == "--desktop");
    if headless && desktop {
        return Err("--headless is a demo check; use --desktop for desktop sessions".into());
    }
    let addr = std::env::args()
        .skip(1)
        .find(|arg| arg != "--headless" && arg != "--desktop")
        .unwrap_or_else(|| {
            std::env::var("WAYTRANSFER_ADDRESS").unwrap_or_else(|_| "127.0.0.1:7878".into())
        });
    let mut stream = TcpStream::connect(addr)?;
    handshake(&mut stream)?;
    if headless {
        let mut decoder = Decoder::default();
        decoder.apply(read_message(&mut stream)?)?;
        write_message(
            &mut stream,
            &Message::Pointer {
                x: 40,
                y: 40,
                pressed: true,
            },
        )?;
        decoder.apply(read_message(&mut stream)?)?;
        if decoder.pixels != demo_frame(40, 40, true) {
            return Err("reconstruction differs".into());
        }
        println!("Demo headless round trip passed");
        return Ok(());
    }
    let mut reader = stream.try_clone()?;
    let (tx, rx) = mpsc::sync_channel(2);
    thread::spawn(move || loop {
        let message = read_message(&mut reader);
        let failed = message.is_err();
        if tx.send(message).is_err() || failed {
            break;
        }
    });
    let mut writer = stream.try_clone()?;
    let (input_tx, input_rx) = mpsc::sync_channel(128);
    thread::spawn(move || {
        for message in input_rx {
            if write_message(&mut writer, &message).is_err() {
                break;
            }
        }
    });
    let mut scene = client_scene::ClientScene::default();
    let mut native: BTreeMap<u32, NativeWindow> = BTreeMap::new();
    let mut pending = VecDeque::new();
    let mut focused = None;
    println!("Connected; waiting for application windows. Ctrl-C ends the viewer.");
    loop {
        let tick = Instant::now();
        for _ in 0..32 {
            match rx.try_recv() {
                Ok(Err(_)) if native.is_empty() && !scene.closing.is_empty() => return Ok(()),
                Ok(message) => {
                    scene.apply(message?)?;
                }
                Err(mpsc::TryRecvError::Disconnected) => return Err("sender disconnected".into()),
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        let gone: Vec<_> = native
            .keys()
            .filter(|id| !scene.windows.contains_key(id))
            .copied()
            .collect();
        for id in gone {
            native.remove(&id);
        }
        for (&id, surface) in &scene.windows {
            if surface.ready {
                if let std::collections::btree_map::Entry::Vacant(entry) = native.entry(id) {
                    let mut window = Window::new(
                        &surface.title,
                        surface.decoder.size.width as usize,
                        surface.decoder.size.height as usize,
                        WindowOptions {
                            resize: true,
                            ..Default::default()
                        },
                    )?;
                    // Pump all windows once per shared tick; no per-window frame limiter.
                    window.set_target_fps(0);
                    let resize = viewer_input::ResizeTracker::new(
                        window.get_size(),
                        surface.decoder.size,
                        Instant::now(),
                    );
                    entry.insert(NativeWindow {
                        window,
                        resize,
                        pointer: None,
                        keys: Vec::new(),
                        buttons: (false, false),
                    });
                }
                let client = native.get_mut(&id).unwrap();
                client.window.set_title(&surface.title);
                if client.window.is_open() {
                    client.window.update_with_buffer(
                        &surface.decoder.pixels,
                        surface.decoder.size.width as usize,
                        surface.decoder.size.height as usize,
                    )?;
                }
            }
        }
        for (&id, client) in &native {
            let parent = scene.windows[&id]
                .parent
                .and_then(|parent| native.get(&parent))
                .map(|parent| &parent.window);
            viewer_input::attach_parent(&client.window, parent);
        }
        let closed: Vec<_> = native
            .iter()
            .filter(|(_, client)| !client.window.is_open())
            .map(|(id, _)| *id)
            .collect();
        for id in closed {
            pending.push_back(scoped(scene.multi, id, Message::Close));
            native.remove(&id);
            scene.close_locally(id);
        }
        if !scene.multi && !scene.closing.is_empty() && native.is_empty() {
            stream.shutdown(Shutdown::Both)?;
            return Ok(());
        }
        let next_focus = native
            .iter()
            .find(|(_, client)| viewer_input::window_focused(&client.window))
            .map(|(id, _)| *id);
        if next_focus != focused {
            if let Some(id) = focused {
                pending.push_back(scoped(scene.multi, id, Message::Focus { active: false }));
            }
            if let Some(id) = next_focus {
                pending.push_back(scoped(scene.multi, id, Message::Focus { active: true }));
                let client = native.get_mut(&id).unwrap();
                client.pointer = None;
                client.keys.clear();
                client.buttons = (false, false);
            }
            focused = next_focus;
        }
        for (&id, client) in &mut native {
            if let Some(size) = client
                .resize
                .observe(client.window.get_size(), Instant::now())
            {
                pending.retain(|message|!matches!(message,Message::ForWindow {id:other,message} if *other==id && matches!(message.as_ref(),Message::Resize {..})));
                pending.push_back(scoped(scene.multi, id, Message::Resize { size }));
            }
            if focused != Some(id) {
                continue;
            }
            let decoder = &scene.windows[&id].decoder;
            if let Some((x, y)) = client.window.get_unscaled_mouse_pos(MouseMode::Clamp) {
                let (x, y) = if desktop {
                    viewer_input::map_desktop_pointer(x, y, client.window.get_size())
                } else {
                    viewer_input::map_pointer(x, y, client.window.get_size(), decoder.size)
                };
                let state = (x, y, client.window.get_mouse_down(MouseButton::Left));
                if client.pointer != Some(state) {
                    pending.push_back(scoped(
                        scene.multi,
                        id,
                        Message::Pointer {
                            x,
                            y,
                            pressed: state.2,
                        },
                    ));
                    client.pointer = Some(state);
                }
            }
            let buttons = (
                client.window.get_mouse_down(MouseButton::Right),
                client.window.get_mouse_down(MouseButton::Middle),
            );
            if buttons != client.buttons {
                pending.push_back(scoped(
                    scene.multi,
                    id,
                    Message::Buttons {
                        right: buttons.0,
                        middle: buttons.1,
                    },
                ));
                client.buttons = buttons;
            }
            let mut keys: Vec<_> = client
                .window
                .get_keys()
                .into_iter()
                .filter_map(viewer_input::evdev)
                .collect();
            keys.sort_unstable();
            if keys != client.keys {
                pending.push_back(scoped(
                    scene.multi,
                    id,
                    Message::Keys { held: keys.clone() },
                ));
                client.keys = keys;
            }
            if let Some((x, y)) = client.window.get_scroll_wheel() {
                pending.push_back(scoped(
                    scene.multi,
                    id,
                    Message::Scroll {
                        horizontal: (-x * 15.0) as i16,
                        vertical: (-y * 15.0) as i16,
                    },
                ));
            }
        }
        while let Some(message) = pending.pop_front() {
            match input_tx.try_send(message) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(message)) => {
                    pending.push_front(message);
                    break;
                }
                Err(_) => return Err("input transport disconnected".into()),
            }
        }
        if pending.len() > 256 {
            return Err("Input backlog exceeded limit; reconnect when network recovers".into());
        }
        thread::sleep(Duration::from_millis(16).saturating_sub(tick.elapsed()));
    }
}
