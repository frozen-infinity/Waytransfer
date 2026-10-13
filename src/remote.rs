//! A single viewer session; only raw snapshots may be coalesced, never encoded deltas.
use crate::{handshake, read_message_limit, write_message, Encoder, Message, HEIGHT, WIDTH};
use std::{
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
struct Shared {
    frame: Mutex<(u64, Vec<u32>)>,
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
impl RemoteServer {
    pub fn bind(address: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(address)?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            frame: Mutex::new((1, vec![0x18202c; WIDTH * HEIGHT])),
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
                    // BSD/macOS can inherit the listener's nonblocking flag.
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
                            let valid = match &message {
                                Message::Pointer { x, y, .. } => {
                                    (*x as usize) < WIDTH && (*y as usize) < HEIGHT
                                }
                                Message::Keys { held } => {
                                    held.len() <= 128
                                        && held.iter().all(|key| (1..=127).contains(key))
                                }
                                Message::Buttons { .. }
                                | Message::Scroll { .. }
                                | Message::Focus { .. } => true,
                                _ => false,
                            };
                            if !valid {
                                break;
                            }
                            if reader_tx.send(RemoteEvent::Input(message)).is_err() {
                                break;
                            }
                        }
                        reader_connected.store(false, Ordering::Relaxed);
                        reader_shared.changed.notify_all();
                    });
                    let mut encoder = Encoder::default();
                    let mut revision = 0;
                    let output = (|| -> io::Result<()> {
                        while connected.load(Ordering::Relaxed)
                            && !worker_shared.stop.load(Ordering::Relaxed)
                        {
                            let mut frame = worker_shared.frame.lock().unwrap();
                            while frame.0 == revision
                                && connected.load(Ordering::Relaxed)
                                && !worker_shared.stop.load(Ordering::Relaxed)
                            {
                                frame = worker_shared
                                    .changed
                                    .wait_timeout(frame, Duration::from_millis(100))
                                    .unwrap()
                                    .0;
                            }
                            if !connected.load(Ordering::Relaxed)
                                || worker_shared.stop.load(Ordering::Relaxed)
                            {
                                break;
                            }
                            revision = frame.0;
                            let pixels = frame.1.clone();
                            drop(frame);
                            if let Some(message) = encoder.encode(&pixels)? {
                                let bytes = write_message(&mut stream, &message)?;
                                println!(
                                    "Update: {bytes} bytes (raw frame: {})",
                                    WIDTH * HEIGHT * 4
                                );
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
        if pixels.len() != WIDTH * HEIGHT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "wrong frame size",
            ));
        }
        let mut frame = self.shared.frame.lock().unwrap();
        if frame.1 != pixels {
            frame.0 += 1;
            frame.1 = pixels;
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
        // Dropping the receiver unblocks input workers even if its bounded queue is full.
        let (_tx, empty) = mpsc::channel();
        self.events = empty;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
