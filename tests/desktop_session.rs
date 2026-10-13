#![cfg(all(target_os = "linux", feature = "desktop"))]
use std::{
    fs,
    net::{TcpListener, TcpStream},
    os::unix::{fs::FileTypeExt, fs::PermissionsExt},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use waytransfer::{client_scene::ClientScene, *};
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn read_scene(viewer: &mut TcpStream, scene: &mut ClientScene, log: &std::path::Path) {
    scene
        .apply(read_message(viewer).unwrap_or_else(|error| {
            panic!(
                "{error}; {}; {}",
                fs::read_to_string(log).unwrap_or_default(),
                sway_state(log.parent().unwrap())
            )
        }))
        .unwrap();
}
fn send(viewer: &mut TcpStream, message: Message) {
    write_message(
        viewer,
        &Message::ForWindow {
            id: 1,
            message: Box::new(message),
        },
    )
    .unwrap();
}
fn keys(viewer: &mut TcpStream, held: Vec<u16>) {
    send(viewer, Message::Keys { held });
}
fn type_command(viewer: &mut TcpStream, command: &str) {
    for c in command.chars() {
        let key = match c {
            'a' => 30,
            'b' => 48,
            'c' => 46,
            'd' => 32,
            'e' => 18,
            'f' => 33,
            'g' => 34,
            'h' => 35,
            'i' => 23,
            'j' => 36,
            'k' => 37,
            'l' => 38,
            'm' => 50,
            'n' => 49,
            'o' => 24,
            'p' => 25,
            'q' => 16,
            'r' => 19,
            's' => 31,
            't' => 20,
            'u' => 22,
            'v' => 47,
            'w' => 17,
            'x' => 45,
            'y' => 21,
            'z' => 44,
            ' ' => 57,
            '-' => 12,
            '\n' => 28,
            _ => panic!("unsupported char"),
        };
        keys(viewer, vec![key]);
        keys(viewer, vec![]);
        thread::sleep(Duration::from_millis(15));
    }
}
fn connect(address: std::net::SocketAddr) -> TcpStream {
    let end = Instant::now() + Duration::from_secs(15);
    let mut viewer = loop {
        if let Ok(stream) = TcpStream::connect(address) {
            break stream;
        }
        assert!(Instant::now() < end, "desktop listener timeout");
        thread::sleep(Duration::from_millis(20));
    };
    handshake(&mut viewer).unwrap();
    viewer
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    viewer
}
#[test]
fn whole_desktop_shortcuts_typing_pointer_resize_and_reconnect() {
    if std::env::var("WAYTRANSFER_TEST_DESKTOP").as_deref() != Ok("1") {
        return;
    }
    let runtime = tempfile::tempdir().unwrap();
    fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let config = runtime.path().join("sway.conf");
    let log = runtime.path().join("desktop.log");
    fs::write(&config,format!("output * mode 640x480\noutput * bg #112233 solid_color\nfont pango:monospace 12\ndefault_border pixel 2\ngaps inner 4\ngaps outer 8\nseat seat0 fallback true\nbindsym Mod4+Return exec weston-terminal\nbindsym Mod4+1 workspace 1\nbindsym Mod4+2 workspace 2\nbindsym --whole-window button3 exec touch {}/desktop-pointer-ok\n",runtime.path().display())).unwrap();
    let sway_log = fs::File::create(runtime.path().join("sway.log")).unwrap();
    let mut sway = ChildGuard(
        Command::new("sway")
            .args(["--config", config.to_str().unwrap()])
            .current_dir(runtime.path())
            .env("XDG_RUNTIME_DIR", runtime.path())
            .env("HOME", runtime.path())
            .env("SHELL", "/bin/sh")
            .env("WLR_BACKENDS", "headless")
            .env("WLR_HEADLESS_OUTPUTS", "1")
            .env("WLR_RENDERER", "pixman")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .stdout(Stdio::from(sway_log.try_clone().unwrap()))
            .stderr(Stdio::from(sway_log))
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let socket = loop {
        let candidate = fs::read_dir(runtime.path())
            .unwrap()
            .flatten()
            .find(|entry| {
                entry.file_name().to_string_lossy().starts_with("wayland-")
                    && entry.file_type().is_ok_and(|t| t.is_socket())
            });
        if let Some(candidate) = candidate {
            break candidate.path();
        }
        assert!(
            Instant::now() < deadline && sway.0.try_wait().unwrap().is_none(),
            "Sway startup failed: {}",
            fs::read_to_string(runtime.path().join("sway.log")).unwrap()
        );
        thread::sleep(Duration::from_millis(20));
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let out = fs::File::create(&log).unwrap();
    let mut desktop = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_desktop"))
            .args([
                "--listen",
                &address.to_string(),
                "--output",
                "HEADLESS-1",
                "--fps",
                "10",
                "--size",
                "640x480",
            ])
            .env("XDG_RUNTIME_DIR", runtime.path())
            .env("WAYLAND_DISPLAY", &socket)
            .env_remove("WAYLAND_SOCKET")
            .stdout(Stdio::from(out.try_clone().unwrap()))
            .stderr(Stdio::from(out))
            .spawn()
            .unwrap(),
    );
    let mut viewer = connect(address);
    let mut scene = ClientScene::default();
    let deadline = Instant::now() + Duration::from_secs(15);
    while scene
        .windows
        .get(&1)
        .is_none_or(|w| !w.ready || w.decoder.pixels[0] != 0x112233)
    {
        assert!(
            Instant::now() < deadline && desktop.0.try_wait().unwrap().is_none(),
            "capture failed: {}",
            fs::read_to_string(&log).unwrap()
        );
        read_scene(&mut viewer, &mut scene, &log);
    }
    assert_eq!(
        scene.windows[&1].decoder.size,
        Dimensions::new(640, 480).unwrap()
    );
    send(&mut viewer, Message::Focus { active: true });
    keys(&mut viewer, vec![125, 28]);
    keys(&mut viewer, vec![]);
    let deadline = Instant::now() + Duration::from_secs(15);
    while background_fraction(&scene) > 0.9 {
        assert!(
            Instant::now() < deadline,
            "compositor shortcut did not launch terminal"
        );
        read_scene(&mut viewer, &mut scene, &log);
    }
    // The virtual input owns the complete desktop, not only a hosted surface.
    thread::sleep(Duration::from_millis(250));
    type_command(&mut viewer, "touch desktop-input-ok\n");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !runtime.path().join("desktop-input-ok").exists() {
        assert!(
            Instant::now() < deadline,
            "desktop typing failed: {}",
            fs::read_to_string(&log).unwrap()
        );
        thread::sleep(Duration::from_millis(20));
    }
    send(
        &mut viewer,
        Message::Pointer {
            x: 512,
            y: 512,
            pressed: false,
        },
    );
    send(
        &mut viewer,
        Message::Buttons {
            right: true,
            middle: false,
        },
    );
    send(
        &mut viewer,
        Message::Buttons {
            right: false,
            middle: false,
        },
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !runtime.path().join("desktop-pointer-ok").exists() {
        assert!(Instant::now() < deadline, "desktop pointer binding failed");
        thread::sleep(Duration::from_millis(20));
    }
    eprintln!("desktop phase: workspace 2");
    keys(&mut viewer, vec![125, 3]);
    keys(&mut viewer, vec![]);
    while background_fraction(&scene) < 0.9 {
        read_scene(&mut viewer, &mut scene, &log);
    }
    eprintln!("desktop phase: workspace 1");
    keys(&mut viewer, vec![125, 2]);
    keys(&mut viewer, vec![]);
    while background_fraction(&scene) > 0.9 {
        read_scene(&mut viewer, &mut scene, &log);
    }
    send(
        &mut viewer,
        Message::Resize {
            size: Dimensions::new(320, 240).unwrap(),
        },
    );
    while scene.windows[&1].decoder.size != Dimensions::new(320, 240).unwrap() {
        read_scene(&mut viewer, &mut scene, &log);
    }
    // The same normalized position remains usable after downsampling the stream.
    fs::remove_file(runtime.path().join("desktop-pointer-ok")).unwrap();
    send(
        &mut viewer,
        Message::Pointer {
            x: 512,
            y: 512,
            pressed: false,
        },
    );
    send(
        &mut viewer,
        Message::Buttons {
            right: true,
            middle: false,
        },
    );
    send(
        &mut viewer,
        Message::Buttons {
            right: false,
            middle: false,
        },
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !runtime.path().join("desktop-pointer-ok").exists() {
        assert!(
            Instant::now() < deadline,
            "pointer failed after downsampling"
        );
        thread::sleep(Duration::from_millis(20));
    }
    keys(&mut viewer, vec![42]);
    drop(viewer);
    thread::sleep(Duration::from_millis(200));
    let mut viewer = connect(address);
    let mut scene = ClientScene::default();
    while scene.windows.get(&1).is_none_or(|w| !w.ready) {
        read_scene(&mut viewer, &mut scene, &log);
    }
    send(&mut viewer, Message::Focus { active: true });
    type_command(&mut viewer, "touch desktop-reconnect-ok\n");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !runtime.path().join("desktop-reconnect-ok").exists() {
        assert!(Instant::now() < deadline, "disconnect left Shift held");
        thread::sleep(Duration::from_millis(20));
    }
    send(&mut viewer, Message::Close);
    let deadline = Instant::now() + Duration::from_secs(10);
    // Frames already queued before Close are valid; the connection must then end.
    while read_message(&mut viewer).is_ok() {
        assert!(
            Instant::now() < deadline,
            "desktop Close did not disconnect"
        );
    }
    assert!(desktop.0.try_wait().unwrap().is_none());
}

fn sway_state(runtime: &std::path::Path) -> String {
    let socket = fs::read_dir(runtime).unwrap().flatten().find(|entry| {
        entry.file_name().to_string_lossy().starts_with("sway-ipc")
            && entry.file_type().is_ok_and(|t| t.is_socket())
    });
    let Some(socket) = socket else {
        return "no Sway IPC socket".into();
    };
    ["get_workspaces", "get_tree"]
        .into_iter()
        .map(|kind| {
            let output = Command::new("swaymsg")
                .args(["-s", socket.path().to_str().unwrap(), "-t", kind])
                .output()
                .unwrap();
            format!(
                "{kind}: {}",
                String::from_utf8_lossy(&output.stdout)
                    .chars()
                    .take(8000)
                    .collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

// A software cursor may be present even without requesting cursor overlay. Require
// the wallpaper to dominate an empty workspace, not every individual pixel.
fn background_fraction(scene: &ClientScene) -> f64 {
    let pixels = &scene.windows[&1].decoder.pixels;
    pixels.iter().filter(|pixel| **pixel == 0x112233).count() as f64 / pixels.len() as f64
}
