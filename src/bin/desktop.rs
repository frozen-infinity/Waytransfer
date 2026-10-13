#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Desktop capture runs on Linux; use the viewer on macOS.");
    std::process::exit(1);
}
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    use wayland_client::Connection;
    use waytransfer::{
        desktop::{self, DesktopState},
        remote::{RemoteEvent, RemoteServer, WindowSnapshot},
        Dimensions, Message,
    };
    let mut address = "127.0.0.1:7878".to_string();
    let mut output = None;
    let mut fps = 15u32;
    let mut view_only = false;
    let mut list = false;
    let mut layout = "us".to_string();
    let mut bound = Dimensions::new(1920, 1080)?;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => address = args.next().ok_or("--listen needs address")?,
            "--output" => output = Some(args.next().ok_or("--output needs name")?),
            "--fps" => {
                fps = args.next().ok_or("--fps needs 1-60")?.parse()?;
                if !(1..=60).contains(&fps) {
                    return Err("--fps needs 1-60".into());
                }
            }
            "--size" => bound = args.next().ok_or("--size needs WIDTHxHEIGHT")?.parse()?,
            "--layout" => layout = args.next().ok_or("--layout needs XKB layout")?,
            "--view-only" => view_only = true,
            "--list-outputs" => list = true,
            "--help" | "-h" => {
                println!("desktop [--listen 127.0.0.1:7878] [--output NAME] [--fps 15] [--size 1920x1080] [--layout us] [--view-only] [--list-outputs]\nConnects to WAYLAND_DISPLAY; captures one complete output. Use scripts/headless-hyprland.sh for a separate Hyprland session.");
                return Ok(());
            }
            _ => return Err(format!("Unknown option: {arg}").into()),
        }
    }
    let connection = Connection::connect_to_env()?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    let mut state = DesktopState::default();
    connection.display().get_registry(&qh, ());
    queue.roundtrip(&mut state)?;
    queue.roundtrip(&mut state)?;
    if list {
        for output in &state.outputs {
            println!("{}: {}x{}", output.name, output.width, output.height);
        }
        return Ok(());
    }
    let name = state.select(output.as_deref(), view_only, &layout, &qh)?;
    let remote = RemoteServer::bind_multi(&address)?;
    let running = Arc::new(AtomicBool::new(true));
    let signal = running.clone();
    ctrlc::set_handler(move || signal.store(false, Ordering::Relaxed))?;
    println!(
        "Desktop output {name}; viewer address {}; {}",
        remote.local_addr(),
        if view_only {
            "view only"
        } else {
            "virtual keyboard/pointer enabled"
        }
    );
    let start = Instant::now();
    let mut size = bound;
    let mut next = Instant::now();
    let mut pending_since = Instant::now();
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        while running.load(Ordering::Relaxed) {
            for _ in 0..128 {
                match remote.events.try_recv() {
                    Ok(RemoteEvent::Disconnected) => state.release(desktop::time(start)),
                    Ok(RemoteEvent::Input(Message::ForWindow { id: 1, message })) => match *message
                    {
                        Message::Resize { size: requested } => {
                            bound = requested;
                        }
                        Message::Close => {
                            state.release(desktop::time(start));
                            remote.disconnect_viewer()?;
                        }
                        input => state.input(input, size, desktop::time(start)),
                    },
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            if !state.pending && Instant::now() >= next {
                state.request_capture(&qh);
                pending_since = Instant::now();
                next = Instant::now() + Duration::from_secs_f64(1.0 / f64::from(fps));
            }
            desktop::pump(
                &connection,
                &mut queue,
                &mut state,
                Duration::from_millis(5),
            )?;
            if let Some(error) = state.error.take() {
                return Err(error.into());
            }
            if state.pending && pending_since.elapsed() > Duration::from_secs(10) {
                return Err(
                    "Capture timed out; check headless output and capture permission".into(),
                );
            }
            if let Some((dimensions, pixels)) = state.take_frame(bound)? {
                size = dimensions;
                remote.publish_windows(vec![WindowSnapshot {
                    id: 1,
                    parent: None,
                    title: format!("Waytransfer desktop: {name}"),
                    size,
                    pixels: Arc::new(pixels),
                }])?;
            }
        }
        Ok(())
    })();
    state.release(desktop::time(start));
    state.destroy_input();
    let _ = connection.flush();
    result
}
