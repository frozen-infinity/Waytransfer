use minifb::{MouseButton, MouseMode, Window, WindowOptions};
use std::{
    collections::VecDeque,
    error::Error,
    net::{Shutdown, TcpStream},
    sync::mpsc,
    thread,
};
use waytransfer::*;
fn main() -> Result<(), Box<dyn Error>> {
    let headless = std::env::args().any(|arg| arg == "--headless");
    let addr = std::env::args()
        .skip(1)
        .find(|arg| arg != "--headless")
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
        println!("TCP handshake, initial frame, pointer input and delta reconstruction passed");
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
    // Native macOS window creation stays on the main thread.
    let mut window = Window::new(
        "Waytransfer — close window to disconnect",
        WIDTH,
        HEIGHT,
        WindowOptions::default(),
    )?;
    window.set_target_fps(60);
    // Input writes use a bounded queue so a slow network cannot freeze the UI.
    let mut writer = stream.try_clone()?;
    let (input_tx, input_rx) = mpsc::sync_channel(128);
    thread::spawn(move || {
        for message in input_rx {
            if write_message(&mut writer, &message).is_err() {
                break;
            }
        }
    });
    let mut decoder = Decoder::default();
    let mut previous = None;
    let mut keys = Vec::new();
    let mut buttons = (false, false);
    let mut active = None;
    let mut pending = VecDeque::new();
    while window.is_open() {
        match rx.try_recv() {
            Ok(message) => decoder.apply(message?)?,
            Err(mpsc::TryRecvError::Disconnected) => return Err("sender disconnected".into()),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        let focused = viewer_input::window_focused(&window);
        if active != Some(focused) {
            pending.push_back(Message::Focus { active: focused });
            active = Some(focused);
            previous = None;
            keys.clear();
            buttons = (false, false);
        }
        if focused {
            if let Some((x, y)) = window.get_mouse_pos(MouseMode::Clamp) {
                let state = (x as u16, y as u16, window.get_mouse_down(MouseButton::Left));
                if previous != Some(state) {
                    // Coalesce motion only; preserve every observed button transition.
                    if matches!(pending.back(), Some(Message::Pointer { pressed, .. }) if *pressed == state.2)
                    {
                        pending.pop_back();
                    }
                    pending.push_back(Message::Pointer {
                        x: state.0,
                        y: state.1,
                        pressed: state.2,
                    });
                    previous = Some(state);
                }
            }
            let next_buttons = (
                window.get_mouse_down(MouseButton::Right),
                window.get_mouse_down(MouseButton::Middle),
            );
            if buttons != next_buttons {
                pending.push_back(Message::Buttons {
                    right: next_buttons.0,
                    middle: next_buttons.1,
                });
                buttons = next_buttons;
            }
            let mut next_keys: Vec<_> = window
                .get_keys()
                .into_iter()
                .filter_map(viewer_input::evdev)
                .collect();
            next_keys.sort_unstable();
            if keys != next_keys {
                pending.push_back(Message::Keys {
                    held: next_keys.clone(),
                });
                keys = next_keys;
            }
            if let Some((x, y)) = window.get_scroll_wheel() {
                pending.push_back(Message::Scroll {
                    horizontal: (-x * 15.0) as i16,
                    vertical: (-y * 15.0) as i16,
                });
            }
        }
        while let Some(message) = pending.pop_front() {
            match input_tx.try_send(message) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(message)) => {
                    pending.push_front(message);
                    break;
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err("input transport disconnected".into())
                }
            }
        }
        if pending.len() > 256 {
            return Err("Input backlog exceeded limit; reconnect when network recovers".into());
        }
        window.update_with_buffer(&decoder.pixels, WIDTH, HEIGHT)?;
    }
    stream.shutdown(Shutdown::Both)?;
    Ok(())
}
