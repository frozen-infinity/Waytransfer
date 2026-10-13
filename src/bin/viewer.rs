use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};
use std::{
    error::Error,
    net::{Shutdown, TcpStream},
    sync::mpsc,
    thread,
    time::Duration,
};
use waytransfer::*;
fn main() -> Result<(), Box<dyn Error>> {
    let headless = std::env::args().any(|arg| arg == "--headless");
    let addr = std::env::args()
        .skip(1)
        .find(|arg| arg != "--headless")
        .unwrap_or_else(|| "127.0.0.1:7878".into());
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
        "Waytransfer demo — Esc to close",
        WIDTH,
        HEIGHT,
        WindowOptions::default(),
    )?;
    window.set_target_fps(60);
    // Input writes use a bounded queue so a slow network cannot freeze the UI.
    let mut writer = stream.try_clone()?;
    let (input_tx, input_rx) = mpsc::sync_channel(2);
    thread::spawn(move || {
        for message in input_rx {
            if write_message(&mut writer, &message).is_err() {
                break;
            }
        }
    });
    let mut decoder = Decoder::default();
    let mut previous = None;
    while window.is_open() && !window.is_key_down(Key::Escape) {
        match rx.try_recv() {
            Ok(message) => decoder.apply(message?)?,
            Err(mpsc::TryRecvError::Disconnected) => return Err("sender disconnected".into()),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if let Some((x, y)) = window.get_mouse_pos(MouseMode::Clamp) {
            let state = (x as u16, y as u16, window.get_mouse_down(MouseButton::Left));
            if previous != Some(state) {
                match input_tx.try_send(Message::Pointer {
                    x: state.0,
                    y: state.1,
                    pressed: state.2,
                }) {
                    Ok(()) => previous = Some(state),
                    Err(mpsc::TrySendError::Full(_)) => {}
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        return Err("input transport disconnected".into())
                    }
                }
            }
        }
        window.update_with_buffer(&decoder.pixels, WIDTH, HEIGHT)?;
    }
    stream.shutdown(Shutdown::Both)?;
    thread::sleep(Duration::from_millis(1));
    Ok(())
}
