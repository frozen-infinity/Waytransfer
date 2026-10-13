//! Manual native GUI regression: focus/type in both windows, resize one, close one.
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    sync::Arc,
    time::{Duration, Instant},
};
use waytransfer::{
    remote::{RemoteEvent, RemoteServer, WindowSnapshot},
    *,
};
fn main() -> Result<(), Box<dyn Error>> {
    let server = RemoteServer::bind_multi("127.0.0.1:17879")?;
    let mut windows: BTreeMap<_, _> = (1..=2)
        .map(|id| {
            let size = Dimensions::new(400, 250).unwrap();
            (
                id,
                WindowSnapshot {
                    id,
                    parent: None,
                    title: format!("Waytransfer probe {id}"),
                    size,
                    pixels: Arc::new(
                        demo_frame_sized(size, 100 * id as usize, 100, false).unwrap(),
                    ),
                },
            )
        })
        .collect();
    server.publish_windows(windows.values().cloned().collect())?;
    println!(
        "Connect viewer to {}; press/release A in each window, resize one, close one",
        server.local_addr()
    );
    let mut focused = None;
    let mut pressed = BTreeSet::new();
    let mut released = BTreeSet::new();
    let mut resized = false;
    let mut closed = false;
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        if let Ok(RemoteEvent::Input(Message::ForWindow { id, message })) =
            server.events.recv_timeout(Duration::from_millis(100))
        {
            match *message {
                Message::Focus { active: true } => {
                    focused = Some(id);
                    println!("focus {id}");
                }
                Message::Focus { active: false } => {
                    if focused == Some(id) {
                        focused = None;
                    }
                }
                Message::Keys { held } => {
                    if focused != Some(id) {
                        return Err("input sent to unfocused window".into());
                    }
                    if held.contains(&30) {
                        pressed.insert(id);
                        println!("A pressed {id}");
                    } else if pressed.contains(&id) {
                        released.insert(id);
                        println!("A released {id}");
                    }
                }
                Message::Resize { size } => {
                    if let Some(window) = windows.get_mut(&id) {
                        window.size = size;
                        window.pixels =
                            Arc::new(demo_frame_sized(size, 100 * id as usize, 100, false)?);
                        resized = true;
                        println!("resize {id}: {}x{}", size.width, size.height);
                    }
                }
                Message::Close => {
                    windows.remove(&id);
                    closed = true;
                    println!("close {id}; {} surviving window(s)", windows.len());
                }
                _ => {}
            }
            server.publish_windows(windows.values().cloned().collect())?;
        }
        if released.len() == 2 && resized && closed && windows.len() == 1 {
            println!(
                "PASS: independent native focus/key input, resize and close with surviving window"
            );
            return Ok(());
        }
    }
    Err("Native multiwindow probe timed out".into())
}
