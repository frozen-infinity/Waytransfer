#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The compositor runs on Linux; run the viewer on macOS.");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use smithay::reexports::wayland_server::ListeningSocket;
    use std::{
        process::{Child, Command},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread,
        time::{Duration, Instant},
    };
    use waytransfer::{
        compositor::SingleWindow,
        remote::{RemoteEvent, RemoteServer},
    };
    struct ChildGuard(Option<Child>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(child) = self.0.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    let mut address = "127.0.0.1:7878".to_string();
    let mut socket_name = "waytransfer-0".to_string();
    let mut command = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => address = args.next().ok_or("--listen needs an address")?,
            "--socket" => socket_name = args.next().ok_or("--socket needs a name")?,
            "--" => {
                command.extend(args);
                break;
            }
            "--help" | "-h" => {
                println!("compositor [--listen 127.0.0.1:7878] [--socket waytransfer-0] [-- PROGRAM ARGS...]\nHeadless single-window Wayland session; wl_shm software clients only.");
                return Ok(());
            }
            _ => {
                return Err(
                    format!("Unknown option: {arg}; use -- before application command").into(),
                )
            }
        }
    }
    if socket_name.contains('/') || socket_name.is_empty() {
        return Err("socket name must be a simple filename".into());
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or("XDG_RUNTIME_DIR is unset; run inside your Linux user session")?;
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(&runtime)?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err("XDG_RUNTIME_DIR must be a private directory (mode 0700)".into());
    }
    let socket = ListeningSocket::bind(&socket_name)?;
    let remote = RemoteServer::bind(&address)?;
    let mut display = SingleWindow::new_display()?;
    let dh = display.handle();
    let mut state = SingleWindow::new(&dh)?;
    let (mut renderer, mut image) = SingleWindow::renderer()?;
    let running = Arc::new(AtomicBool::new(true));
    let signal = running.clone();
    ctrlc::set_handler(move || signal.store(false, Ordering::Relaxed))?;
    println!(
        "Single-window session: WAYLAND_DISPLAY={socket_name}; viewer address {}",
        remote.local_addr()
    );
    println!("Software rendering only; applications run in this separate Wayland session.");
    let mut child = ChildGuard(if let Some(program) = command.first() {
        Some(
            Command::new(program)
                .args(&command[1..])
                .env("WAYLAND_DISPLAY", &socket_name)
                .env("GDK_BACKEND", "wayland")
                .env("GSK_RENDERER", "cairo")
                .env("GDK_DEBUG", "gl-disable")
                .env("QT_QPA_PLATFORM", "wayland")
                .env("QT_QUICK_BACKEND", "software")
                .env("QT_WAYLAND_DISABLE_WINDOWDECORATION", "1")
                .spawn()?,
        )
    } else {
        None
    });
    while running.load(Ordering::Relaxed) {
        let tick = Instant::now();
        while let Some(stream) = socket.accept()? {
            state.insert_client(&dh, stream)?;
        }
        display.dispatch_clients(&mut state)?;
        // Bound per-tick input work as well as the network queue.
        for _ in 0..128 {
            match remote.events.try_recv() {
                Ok(RemoteEvent::Input(message)) => {
                    if let Err(e) = state.input(message) {
                        eprintln!("Ignored invalid input: {e}");
                    }
                }
                Ok(RemoteEvent::Disconnected) => state.release_input(),
                Err(_) => break,
            }
        }
        if let Some(pixels) = state.render(&mut renderer, &mut image)? {
            remote.publish(pixels)?;
        }
        display.flush_clients()?;
        if let Some(child) = child.0.as_mut() {
            if let Some(status) = child.try_wait()? {
                println!("Application exited: {status}");
                break;
            }
        }
        thread::sleep(Duration::from_millis(33).saturating_sub(tick.elapsed()));
    }
    Ok(())
}
