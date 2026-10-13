//! Manual native-viewer regression, not a replacement for Linux application tests.
//! Run this, connect the viewer to its printed address, click inside, press A,
//! release it, then switch to another app. The probe checks actual TCP input.
use std::{
    error::Error,
    time::{Duration, Instant},
};
use waytransfer::{
    remote::{RemoteEvent, RemoteServer},
    *,
};
fn main() -> Result<(), Box<dyn Error>> {
    let server = RemoteServer::bind("127.0.0.1:17879")?;
    server.publish(demo_frame(320, 200, false))?;
    println!(
        "Connect viewer to {}; click, press/release A, then switch away",
        server.local_addr()
    );
    let (mut focus, mut click, mut press, mut release, mut blur) =
        (false, false, false, false, false);
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        if let Ok(RemoteEvent::Input(message)) =
            server.events.recv_timeout(Duration::from_millis(100))
        {
            match message {
                Message::Focus { active: true } => {
                    focus = true;
                    println!("focus gained");
                }
                Message::Focus { active: false } if focus => {
                    blur = true;
                    println!("focus lost");
                }
                Message::Keys { held } if held.contains(&30) => {
                    press = true;
                    println!("A pressed");
                }
                Message::Keys { held } if press && !held.contains(&30) => {
                    release = true;
                    println!("A released");
                }
                Message::Pointer { x, y, pressed } => {
                    if pressed {
                        click = true;
                        println!("pointer click");
                    }
                    server.publish(demo_frame(x as usize, y as usize, pressed))?;
                }
                _ => {}
            }
        }
        if focus && click && press && release && blur {
            println!("PASS: native viewer forwarded focus, click, A press/release and focus loss");
            return Ok(());
        }
    }
    Err(format!(
        "Timed out: focus={focus} click={click} press={press} release={release} blur={blur}"
    )
    .into())
}
