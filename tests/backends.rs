#![cfg(all(target_os = "linux", feature = "compositor"))]
use std::{
    fs,
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use waytransfer::{client_scene::ClientScene, *};
struct Session {
    child: Child,
    runtime: tempfile::TempDir,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Session {
    fn log(&self) -> String {
        fs::read_to_string(self.runtime.path().join("session.log")).unwrap_or_default()
    }
}
fn start(flags: &[&str], program: &str, args: &[&str]) -> (Session, TcpStream) {
    let runtime = tempfile::tempdir().unwrap();
    fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let log = fs::File::create(runtime.path().join("session.log")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_compositor"));
    command
        .args(["--listen", &address.to_string(), "--socket", "backend-test"])
        .args(flags)
        .arg("--")
        .arg(program)
        .args(args)
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .env("SHELL", "/bin/sh")
        .env("LIBGL_ALWAYS_SOFTWARE", "1")
        .current_dir(runtime.path())
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log));
    let mut session = Session {
        child: command.spawn().unwrap(),
        runtime,
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut viewer = loop {
        if let Ok(stream) = TcpStream::connect(address) {
            break stream;
        }
        assert!(
            Instant::now() < deadline,
            "startup timeout: {}",
            session.log()
        );
        assert!(
            session.child.try_wait().unwrap().is_none(),
            "compositor exited: {}",
            session.log()
        );
        thread::sleep(Duration::from_millis(20));
    };
    handshake(&mut viewer).unwrap();
    viewer
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    (session, viewer)
}
fn receive(viewer: &mut TcpStream, scene: &mut ClientScene, session: &Session) {
    let message = read_message(viewer).unwrap_or_else(|error| panic!("{error}; {}", session.log()));
    scene.apply(message).unwrap();
}
#[cfg(feature = "x11")]
#[test]
fn real_xterm_renders_and_executes_scoped_remote_input() {
    if std::env::var("WAYTRANSFER_TEST_WESTON").as_deref() != Ok("1") {
        return;
    }
    let (session, mut viewer) = start(
        &["--x11"],
        "xterm",
        &["-title", "Waytransfer X11 test", "-e", "/bin/sh"],
    );
    let mut scene = ClientScene::default();
    while scene.windows.values().all(|window| !window.ready) {
        receive(&mut viewer, &mut scene, &session);
    }
    let id = *scene
        .windows
        .iter()
        .find(|(_, window)| window.ready)
        .unwrap()
        .0;
    assert!(scene.windows[&id].title.contains("X11"));
    // XWayland can commit its blank mapped surface before xterm paints text.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !scene.windows[&id]
        .decoder
        .pixels
        .iter()
        .any(|pixel| *pixel != scene.windows[&id].decoder.pixels[0])
    {
        assert!(
            Instant::now() < deadline,
            "X11 did not paint: {}",
            session.log()
        );
        receive(&mut viewer, &mut scene, &session);
    }
    let send = |viewer: &mut TcpStream, message| {
        write_message(
            viewer,
            &Message::ForWindow {
                id,
                message: Box::new(message),
            },
        )
        .unwrap()
    };
    send(&mut viewer, Message::Focus { active: true });
    for c in "touch x11-ok\n".chars() {
        let key = match c {
            't' => 20,
            'o' => 24,
            'u' => 22,
            'c' => 46,
            'h' => 35,
            ' ' => 57,
            'x' => 45,
            '1' => 2,
            '-' => 12,
            'k' => 37,
            '\n' => 28,
            _ => unreachable!(),
        };
        send(&mut viewer, Message::Keys { held: vec![key] });
        send(&mut viewer, Message::Keys { held: vec![] });
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !session.runtime.path().join("x11-ok").exists() {
        assert!(
            Instant::now() < deadline,
            "X11 input failed: {}",
            session.log()
        );
        thread::sleep(Duration::from_millis(20));
    }
    send(&mut viewer, Message::Close);
    while scene.windows.contains_key(&id) {
        receive(&mut viewer, &mut scene, &session);
    }
}
#[cfg(feature = "gpu")]
#[test]
fn actual_egl_application_produces_changing_frames() {
    if std::env::var("WAYTRANSFER_TEST_GPU").as_deref() != Ok("1") {
        return;
    }
    let (session, mut viewer) = start(&["--gpu"], "weston-simple-egl", &[]);
    let mut scene = ClientScene::default();
    while scene.windows.values().all(|window| !window.ready) {
        receive(&mut viewer, &mut scene, &session);
    }
    let id = *scene
        .windows
        .iter()
        .find(|(_, window)| window.ready)
        .unwrap()
        .0;
    let first = scene.windows[&id].decoder.pixels.clone();
    let deadline = Instant::now() + Duration::from_secs(10);
    while scene.windows[&id].decoder.pixels == first {
        assert!(
            Instant::now() < deadline,
            "EGL client did not animate: {}",
            session.log()
        );
        receive(&mut viewer, &mut scene, &session);
    }
    assert!(scene.windows[&id]
        .decoder
        .pixels
        .iter()
        .any(|pixel| *pixel != scene.windows[&id].decoder.pixels[0]));
}
