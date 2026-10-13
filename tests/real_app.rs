#![cfg(all(target_os = "linux", feature = "compositor"))]
//! Optional local test, mandatory in Linux CI (WAYTRANSFER_TEST_WESTON=1).
use std::{
    fs,
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use waytransfer::*;
struct Session(Child);
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn weston_terminal_renders_and_executes_remote_keyboard_input() {
    if std::env::var("WAYTRANSFER_TEST_WESTON").as_deref() != Ok("1") {
        eprintln!(
            "Set WAYTRANSFER_TEST_WESTON=1 and install weston to run the real terminal smoke test"
        );
        return;
    }
    let runtime = tempfile::tempdir().unwrap();
    fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let log = fs::File::create(runtime.path().join("session.log")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_compositor"))
        .args([
            "--listen",
            &addr.to_string(),
            "--socket",
            "waytransfer-test",
            "--",
            "weston-terminal",
        ])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("HOME", runtime.path())
        .env("SHELL", "/bin/sh")
        .current_dir(runtime.path())
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .unwrap();
    let mut session = Session(child);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut viewer = loop {
        if let Ok(stream) = TcpStream::connect(addr) {
            break stream;
        }
        assert!(
            Instant::now() < deadline,
            "compositor startup timed out: {}",
            fs::read_to_string(runtime.path().join("session.log")).unwrap()
        );
        assert!(
            session.0.try_wait().unwrap().is_none(),
            "compositor exited: {}",
            fs::read_to_string(runtime.path().join("session.log")).unwrap()
        );
        thread::sleep(Duration::from_millis(20));
    };
    handshake(&mut viewer).unwrap();
    viewer
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut decoder = Decoder::default();
    loop {
        decoder.apply(read_message(&mut viewer).unwrap()).unwrap();
        // The terminal produces varied text/background pixels, unlike the waiting screen.
        if decoder.pixels.iter().any(|p| *p != decoder.pixels[0]) {
            break;
        }
        assert!(Instant::now() < deadline, "terminal did not paint");
    }
    write_message(&mut viewer, &Message::Focus { active: true }).unwrap();
    for character in "touch keyboard-ok\n".chars() {
        let key = match character {
            't' => 20,
            'o' => 24,
            'u' => 22,
            'c' => 46,
            'h' => 35,
            ' ' => 57,
            'k' => 37,
            'e' => 18,
            'y' => 21,
            'b' => 48,
            'a' => 30,
            'r' => 19,
            'd' => 32,
            '-' => 12,
            '\n' => 28,
            _ => panic!("unexpected character"),
        };
        write_message(&mut viewer, &Message::Keys { held: vec![key] }).unwrap();
        write_message(&mut viewer, &Message::Keys { held: vec![] }).unwrap();
    }
    while !runtime.path().join("keyboard-ok").exists() {
        assert!(
            Instant::now() < deadline,
            "terminal never executed remote input: {}",
            fs::read_to_string(runtime.path().join("session.log")).unwrap()
        );
        assert!(
            session.0.try_wait().unwrap().is_none(),
            "terminal session exited"
        );
        thread::sleep(Duration::from_millis(20));
    }
    // The shell must see the new terminal dimensions after xdg configure/repaint.
    let size = Dimensions::new(961, 607).unwrap();
    write_message(&mut viewer, &Message::Resize { size }).unwrap();
    while decoder.size != size {
        decoder.apply(read_message(&mut viewer).unwrap()).unwrap();
    }
    thread::sleep(Duration::from_millis(250));
    let before = runtime.path().join("keyboard-ok");
    assert!(before.exists());
    // A second command after resizing verifies that keyboard focus survives it.
    for character in "touch resize-ok\n".chars() {
        let key = match character {
            't' => 20,
            'o' => 24,
            'u' => 22,
            'c' => 46,
            'h' => 35,
            ' ' => 57,
            'r' => 19,
            'e' => 18,
            's' => 31,
            'i' => 23,
            'z' => 44,
            '-' => 12,
            'k' => 37,
            '\n' => 28,
            _ => panic!("unexpected character"),
        };
        write_message(&mut viewer, &Message::Keys { held: vec![key] }).unwrap();
        write_message(&mut viewer, &Message::Keys { held: vec![] }).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !runtime.path().join("resize-ok").exists() {
        assert!(
            Instant::now() < deadline,
            "terminal input did not survive resize"
        );
        thread::sleep(Duration::from_millis(20));
    }
    eprintln!("Real weston-terminal rendered and executed the remote keyboard command");
}
