use std::{error::Error, net::TcpListener};
use waytransfer::*;
fn main() -> Result<(), Box<dyn Error>> {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7878".into());
    let listener = TcpListener::bind(&addr)?;
    println!("Demo listening on {addr}; one client at a time");
    for connection in listener.incoming() {
        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut stream = connection?;
            handshake(&mut stream)?;
            let mut encoder = Encoder::default();
            let mut send =
                |stream: &mut std::net::TcpStream, x, y, pressed| -> Result<(), Box<dyn Error>> {
                    if let Some(frame) = encoder.encode(&demo_frame(x, y, pressed))? {
                        let bytes = write_message(stream, &frame)?;
                        println!("Update: {bytes} bytes (raw frame: {})", WIDTH * HEIGHT * 4);
                    }
                    Ok(())
                };
            send(&mut stream, WIDTH / 2, HEIGHT / 2, false)?;
            loop {
                match read_message(&mut stream)? {
                    Message::Pointer { x, y, pressed }
                        if (x as usize) < WIDTH && (y as usize) < HEIGHT =>
                    {
                        send(&mut stream, x as usize, y as usize, pressed)?
                    }
                    Message::Keys { .. }
                    | Message::Buttons { .. }
                    | Message::Scroll { .. }
                    | Message::Focus { .. } => {}
                    _ => return Err("unexpected client message".into()),
                }
            }
        })();
        if let Err(e) = result {
            eprintln!("Session ended: {e}");
        }
    }
    Ok(())
}
