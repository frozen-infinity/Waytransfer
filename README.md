# Waytransfer

An open-source Rust experiment for using Linux applications from a MacBook over
limited bandwidth. **This initial vertical slice is a synthetic remote display,
not a Wayland compositor, screen capture tool, or application remoting system.**

## Try it

Install stable Rust. On Linux (OMEN) run:

```sh
cargo run --release --bin sender
```

The sender binds loopback by default. Use an SSH tunnel from the MacBook:

```sh
ssh -N -L 7878:127.0.0.1:7878 your-user@your-omen
```

In another MacBook terminal, from this repository:

```sh
cargo run --release --features viewer --bin viewer -- 127.0.0.1:7878
```

Move the pointer inside the window; the sender redraws a square at that location.
Hold the left button to change its color. Escape closes the viewer. Window movement
and display refresh happen locally. The content itself is rasterized on the sender.
There is no font/vector command transport or Metal renderer yet.

Both programs also run on one Mac for a loopback demonstration. Linux sender has
no desktop library dependency. Linux viewer builds require X11/Wayland development
libraries (see CI). Viewer uses minifb's native platform window support.

For a GUI-free end-to-end smoke check with the sender already running:

```sh
cargo run --features viewer --bin viewer -- 127.0.0.1:7878 --headless
```

An optional sender address argument selects the listening interface. Direct network
connections are unencrypted and unauthenticated; use the SSH tunnel for remote use.
The demo serves one client at a time and accepts a new session after disconnect.

## What the slice implements

- Shared versioned messages, bidirectional TCP handshake, length-prefixed JSON.
- 640×400 fixed-size frames split into 32×32 tiles (including partial edge tiles).
- Exact comparison against the previous frame: only changed tiles are sent.
- Lossless LZ4 compression per changed tile; unchanged frames send zero bytes.
- Client retains a framebuffer and applies ordered deltas atomically.
- Pointer movement/button input travels back to the sender. No keyboard injection.
- Bounded packet lengths and decompression output, coordinate/sequence validation,
  bounded viewer queues, separate network workers and native UI thread.

The initial update contains all tiles. Every new connection resets state and gets
a full frame; TCP provides ordering, so no lossy-network recovery protocol is needed
in this version. Each compressed block has an exact expected output size derived
from its coordinates. JSON byte arrays add overhead; this is a readable prototype
wire format, not a final interoperable standard. Protocol v1 has fixed dimensions,
no capability negotiation, and no persistent/content-addressed tile cache.

Sender logs actual encoded bytes per update versus 1,024,000 raw frame bytes.
Tests verify that the synthetic initial frame uses less than 20% of raw bytes and
a one-pixel pointer movement uses less than 10 KB. These are synthetic checks,
not real application benchmarks or promises about wireless latency. RTT still
limits remote input response; moving rendering to the Mac alone cannot remove it.
The sender scans a complete frame per input; optimize CPU/damage tracking later.

## Validation

```sh
cargo fmt --check
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo build --locked --all-features
```

CI runs these on Linux and macOS. Tests cover lossless reconstruction, edge tiles,
idle suppression, small deltas, truncation/oversize packets, invalid updates and a
real loopback TCP exchange with pointer input. GUI behavior requires a manual
window test; CI does not assert rendering or Linux application compatibility.

See [ROADMAP.md](ROADMAP.md). Contributions are welcome; keep changes testable and
report bandwidth measurements with workload, frame size and network conditions.
Licensed under [MIT](LICENSE).
