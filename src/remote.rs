//! One viewer session with independent retained/encoded state per application window.
use crate::{
    handshake, read_message_limit, window_title, write_message, Dimensions, Encoder, Message,
    MAX_DIMENSION, MAX_SCENE_PIXELS, MAX_WINDOWS,
};
use std::{
    collections::BTreeMap,
    io,
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
#[derive(Debug)]
pub enum RemoteEvent {
    Input(Message),
    Disconnected,
}
#[derive(Clone, Debug, PartialEq)]
pub struct WindowSnapshot {
    pub id: u32,
    pub parent: Option<u32>,
    pub title: String,
    pub size: Dimensions,
    pub pixels: Arc<Vec<u32>>,
}
#[derive(Clone, PartialEq)]
struct Scene {
    revision: u64,
    legacy: Option<(Dimensions, Arc<Vec<u32>>)>,
    windows: BTreeMap<u32, WindowSnapshot>,
}
struct Shared {
    scene: Mutex<Scene>,
    changed: Condvar,
    stop: AtomicBool,
    stream: Mutex<Option<TcpStream>>,
}
pub struct RemoteServer {
    shared: Arc<Shared>,
    pub events: Receiver<RemoteEvent>,
    worker: Option<JoinHandle<()>>,
    address: std::net::SocketAddr,
}
fn valid_input(message: &Message, scoped: bool) -> bool {
    match message {
        Message::ForWindow { id, message } if !scoped => *id != 0 && valid_input(message, true),
        Message::Close => scoped,
        Message::Pointer { x, y, .. } => {
            (*x as usize) < MAX_DIMENSION && (*y as usize) < MAX_DIMENSION
        }
        Message::Resize { size } => size.validate().is_ok(),
        Message::Keys { held } => {
            held.len() <= 128 && held.iter().all(|key| (1..=127).contains(key))
        }
        Message::Buttons { .. } | Message::Scroll { .. } | Message::Focus { .. } => true,
        _ => false,
    }
}
impl RemoteServer {
    pub fn bind(address: &str) -> io::Result<Self> {
        Self::bind_sized(address, Dimensions::default())
    }
    pub fn bind_sized(address: &str, size: Dimensions) -> io::Result<Self> {
        let size = size.validate()?;
        Self::start(
            address,
            Some((size, Arc::new(vec![0x18202c; size.pixel_count()]))),
        )
    }
    pub fn bind_multi(address: &str) -> io::Result<Self> {
        Self::start(address, None)
    }
    fn start(address: &str, legacy: Option<(Dimensions, Arc<Vec<u32>>)>) -> io::Result<Self> {
        let listener = TcpListener::bind(address)?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            scene: Mutex::new(Scene {
                revision: 1,
                legacy,
                windows: BTreeMap::new(),
            }),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
            stream: Mutex::new(None),
        });
        let (tx, events) = mpsc::sync_channel(128);
        let worker_shared = shared.clone();
        let worker = thread::spawn(move || {
            while !worker_shared.stop.load(Ordering::Relaxed) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    Err(e) => {
                        eprintln!("Viewer listener: {e}");
                        break;
                    }
                };
                *worker_shared.stream.lock().unwrap() = stream.try_clone().ok();
                let result = (|| -> io::Result<()> {
                    stream.set_nonblocking(false)?;
                    handshake(&mut stream)?;
                    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                    let connected = Arc::new(AtomicBool::new(true));
                    let reader_connected = connected.clone();
                    let mut reader = stream.try_clone()?;
                    let reader_tx = tx.clone();
                    let reader_shared = worker_shared.clone();
                    let input = thread::spawn(move || {
                        while let Ok(message) = read_message_limit(&mut reader, 4096) {
                            if !valid_input(&message, false)
                                || reader_tx.send(RemoteEvent::Input(message)).is_err()
                            {
                                break;
                            }
                        }
                        reader_connected.store(false, Ordering::Relaxed);
                        reader_shared.changed.notify_all();
                    });
                    let mut legacy_encoder = Encoder::default();
                    let mut encoders: BTreeMap<u32, Encoder> = BTreeMap::new();
                    let mut metadata: BTreeMap<u32, (Option<u32>, Dimensions, String)> =
                        BTreeMap::new();
                    let mut revision = 0;
                    let output = (|| -> io::Result<()> {
                        while connected.load(Ordering::Relaxed)
                            && !worker_shared.stop.load(Ordering::Relaxed)
                        {
                            let mut scene = worker_shared.scene.lock().unwrap();
                            while scene.revision == revision
                                && connected.load(Ordering::Relaxed)
                                && !worker_shared.stop.load(Ordering::Relaxed)
                            {
                                scene = worker_shared
                                    .changed
                                    .wait_timeout(scene, Duration::from_millis(100))
                                    .unwrap()
                                    .0;
                            }
                            if !connected.load(Ordering::Relaxed)
                                || worker_shared.stop.load(Ordering::Relaxed)
                            {
                                break;
                            }
                            revision = scene.revision;
                            let snapshot = scene.clone();
                            drop(scene);
                            if let Some((size, pixels)) = &snapshot.legacy {
                                if let Some(frame) = legacy_encoder.encode_sized(*size, pixels)? {
                                    let bytes = write_message(&mut stream, &frame)?;
                                    println!(
                                        "Update: {bytes} bytes (raw frame: {})",
                                        size.pixel_count() * 4
                                    );
                                }
                            }
                            let removed: Vec<_> = metadata
                                .keys()
                                .filter(|id| !snapshot.windows.contains_key(id))
                                .copied()
                                .collect();
                            for id in removed {
                                write_message(&mut stream, &Message::WindowGone { id })?;
                                metadata.remove(&id);
                                encoders.remove(&id);
                            }
                            for (id, window) in &snapshot.windows {
                                let info = (window.parent, window.size, window.title.clone());
                                if metadata.get(id) != Some(&info) {
                                    write_message(
                                        &mut stream,
                                        &Message::WindowInfo {
                                            id: *id,
                                            parent: window.parent,
                                            size: window.size,
                                            title: window.title.clone(),
                                        },
                                    )?;
                                    metadata.insert(*id, info);
                                }
                                if let Some(frame) = encoders
                                    .entry(*id)
                                    .or_default()
                                    .encode_sized(window.size, &window.pixels)?
                                {
                                    let bytes = write_message(
                                        &mut stream,
                                        &Message::ForWindow {
                                            id: *id,
                                            message: Box::new(frame),
                                        },
                                    )?;
                                    println!(
                                        "Window {id}: {bytes} update bytes (raw: {})",
                                        window.size.pixel_count() * 4
                                    );
                                }
                            }
                        }
                        Ok(())
                    })();
                    let _ = stream.shutdown(Shutdown::Both);
                    let _ = input.join();
                    output
                })();
                if let Err(e) = result {
                    eprintln!("Viewer session ended: {e}");
                }
                *worker_shared.stream.lock().unwrap() = None;
                if tx.send(RemoteEvent::Disconnected).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            shared,
            events,
            worker: Some(worker),
            address,
        })
    }
    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.address
    }
    pub fn publish(&self, pixels: Vec<u32>) -> io::Result<()> {
        self.publish_sized(Dimensions::default(), pixels)
    }
    pub fn publish_sized(&self, size: Dimensions, pixels: Vec<u32>) -> io::Result<()> {
        let size = size.validate()?;
        if pixels.len() != size.pixel_count() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "wrong frame size",
            ));
        }
        let mut scene = self.shared.scene.lock().unwrap();
        let value = (size, Arc::new(pixels));
        if scene.legacy.as_ref() != Some(&value) {
            scene.legacy = Some(value);
            scene.revision += 1;
            self.shared.changed.notify_one();
        }
        Ok(())
    }
    pub fn publish_windows(&self, windows: Vec<WindowSnapshot>) -> io::Result<()> {
        if windows.len() > MAX_WINDOWS {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "window limit"));
        }
        let mut next = BTreeMap::new();
        let mut pixels = 0;
        for mut window in windows {
            window.size.validate()?;
            pixels += window.size.pixel_count();
            if window.id == 0
                || window.parent == Some(window.id)
                || window.parent == Some(0)
                || window.pixels.len() != window.size.pixel_count()
                || next.contains_key(&window.id)
                || pixels > MAX_SCENE_PIXELS
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid window snapshot",
                ));
            }
            window.title = window_title(&window.title);
            next.insert(window.id, window);
        }
        let mut scene = self.shared.scene.lock().unwrap();
        if scene.windows != next || scene.legacy.is_some() {
            scene.windows = next;
            scene.legacy = None;
            scene.revision += 1;
            self.shared.changed.notify_one();
        }
        Ok(())
    }
}
impl Drop for RemoteServer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.changed.notify_all();
        if let Some(stream) = self.shared.stream.lock().unwrap().as_ref() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        let (_tx, empty) = mpsc::channel();
        self.events = empty;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
