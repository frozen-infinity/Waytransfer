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
        render_engine::RenderEngine,
        Dimensions,
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
    let mut size = Dimensions::default();
    let mut accelerated = false;
    let mut use_x11 = false;
    let mut command = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--gpu" => accelerated = true,
            "--x11" => use_x11 = true,
            "--listen" => address = args.next().ok_or("--listen needs an address")?,
            "--size" => size = args.next().ok_or("--size needs WIDTHxHEIGHT")?.parse()?,
            "--socket" => socket_name = args.next().ok_or("--socket needs a name")?,
            "--" => {
                command.extend(args);
                break;
            }
            "--help" | "-h" => {
                println!("compositor [--listen 127.0.0.1:7878] [--socket waytransfer-0] [--size 640x400] [-- PROGRAM ARGS...]\nHeadless multiwindow session. Optional --gpu and --x11 require matching build features.");
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
    let remote = RemoteServer::bind_multi(&address)?;
    let mut display = SingleWindow::new_display()?;
    let dh = display.handle();
    let mut state = SingleWindow::new_sized(&dh, size)?;
    let mut renderer = if accelerated {
        #[cfg(feature = "gpu")]
        {
            let renderer = RenderEngine::gpu(&dh)?;
            state.advertise_gpu(&renderer)?;
            renderer
        }
        #[cfg(not(feature = "gpu"))]
        {
            return Err("Build with --features compositor,gpu for --gpu".into());
        }
    } else {
        RenderEngine::software()?
    };
    #[cfg(feature = "x11")]
    let mut event_loop = smithay::reexports::calloop::EventLoop::try_new()?;
    if use_x11 {
        #[cfg(feature = "x11")]
        crate_start_x11(&dh, &event_loop, accelerated)?;
        #[cfg(not(feature = "x11"))]
        return Err("Build with --features compositor,x11 for --x11".into());
    }
    #[cfg(feature = "x11")]
    if use_x11 {
        let deadline = Instant::now() + Duration::from_secs(10);
        while state.x11_display.is_none() {
            display.dispatch_clients(&mut state)?;
            display.flush_clients()?;
            event_loop.dispatch(Duration::from_millis(5), &mut state)?;
            if Instant::now() > deadline {
                return Err("XWayland startup timed out".into());
            }
        }
    }
    let running = Arc::new(AtomicBool::new(true));
    let signal = running.clone();
    ctrlc::set_handler(move || signal.store(false, Ordering::Relaxed))?;
    println!(
        "Multiwindow session: WAYLAND_DISPLAY={socket_name}; viewer address {}",
        remote.local_addr()
    );
    println!(
        "Applications run in this separate Wayland session; renderer: {}",
        if accelerated { "EGL/GLES" } else { "software" }
    );
    let mut child = ChildGuard(if let Some(program) = command.first() {
        let mut launch = Command::new(program);
        launch
            .args(&command[1..])
            .env("WAYLAND_DISPLAY", &socket_name)
            .env("QT_WAYLAND_DISABLE_WINDOWDECORATION", "1");
        if !accelerated {
            launch
                .env("GSK_RENDERER", "cairo")
                .env("GDK_DEBUG", "gl-disable")
                .env("QT_QUICK_BACKEND", "software");
        }
        #[cfg(feature = "x11")]
        if let Some(number) = state.x11_display {
            launch.env("DISPLAY", format!(":{number}"));
        } else {
            launch.env_remove("DISPLAY");
        }
        #[cfg(not(feature = "x11"))]
        launch.env_remove("DISPLAY");
        Some(launch.spawn()?)
    } else {
        None
    });
    while running.load(Ordering::Relaxed) {
        let tick = Instant::now();
        while let Some(stream) = socket.accept()? {
            state.insert_client(&dh, stream)?;
        }
        display.dispatch_clients(&mut state)?;
        #[cfg(feature = "x11")]
        event_loop.dispatch(Duration::ZERO, &mut state)?;
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
        if let Some(windows) = state.render_windows(&mut renderer)? {
            remote.publish_windows(windows)?;
        }
        display.flush_clients()?;
        if let Some(process) = child.0.as_mut() {
            if let Some(status) = process.try_wait()? {
                println!("Application exited: {status}");
                child.0.take();
            }
        }
        thread::sleep(Duration::from_millis(33).saturating_sub(tick.elapsed()));
    }
    Ok(())
}

#[cfg(all(target_os = "linux", feature = "x11"))]
fn crate_start_x11(
    display: &smithay::reexports::wayland_server::DisplayHandle,
    event_loop: &smithay::reexports::calloop::EventLoop<
        'static,
        waytransfer::compositor::SingleWindow,
    >,
) -> Result<(), Box<dyn std::error::Error>> {
    waytransfer::x11::start(display, event_loop, accelerated)
}
