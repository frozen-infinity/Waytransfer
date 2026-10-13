# Waytransfer

Open-source Rust software for using Linux applications from a MacBook over limited
bandwidth. This branch adds an **experimental headless, single-window Wayland
compositor** alongside the original square demo.

A Linux application runs in a separate Wayland session. Its shared-memory buffers
are composed in software on Linux and sent as lossless compressed tile deltas to
the Mac. The Mac displays the result in a native window and sends input back.

## Run a real application

Use stable Rust (1.95 or newer), and build the same branch on both machines.
**Wire protocol v4 requires rebuilding both sides; older v1/v2/v3 viewers cannot
connect.** Stop the original demo sender before starting the compositor on its port.

On Arch Linux / the OMEN, install build/runtime dependencies:

```sh
sudo pacman -S --needed base-devel pkgconf pixman libxkbcommon xkeyboard-config weston
cargo run --release --features compositor --bin compositor -- -- weston-terminal
```

On Debian/Ubuntu, the compositor dependencies are `build-essential pkg-config
libpixman-1-dev libxkbcommon-dev xkb-data weston`. The Linux sender needs no GPU,
DRM device, running desktop compositor or capture permission. `XDG_RUNTIME_DIR`
must point to your private user runtime directory (normally `/run/user/$(id -u)`).
For a headless SSH session that does not provide one, create a private directory:

```sh
mkdir -p /tmp/waytransfer-$(id -u)
chmod 700 /tmp/waytransfer-$(id -u)
export XDG_RUNTIME_DIR=/tmp/waytransfer-$(id -u)
```

From the MacBook, create an SSH tunnel:

```sh
ssh -N -L 7878:127.0.0.1:7878 your-user@your-omen
```

In another MacBook terminal:

```sh
cargo run --release --features viewer --bin viewer -- 127.0.0.1:7878
```

Type into the remote terminal, click, select text or scroll. **Escape now goes to
the application; close the native viewer window to disconnect.** Switching away
from the viewer releases held keys/buttons. Reconnecting gets a fresh full frame.
The application continues while the viewer is disconnected. Ctrl-C in the Linux
compositor terminal ends the session and stops the directly launched application.
When that application exits, the compositor exits too.

Choose another software-rendered Wayland application after `--`:

```sh
cargo run --release --features compositor --bin compositor -- -- your-wayland-app arg1
```

The launcher sets `GDK_BACKEND=wayland`, `GSK_RENDERER=cairo`,
`GDK_DEBUG=gl-disable`, `QT_QPA_PLATFORM=wayland`, `QT_QUICK_BACKEND=software`
and `QT_WAYLAND_DISABLE_WINDOWDECORATION=1` for its child only. These are helpful
software-backend defaults, **not a guarantee that every GTK/Qt application works**.
Avoid applications that forward launches to an already-running instance; they may
open on your existing desktop instead of this session.

For manual launches, start without a command:

```sh
cargo run --release --features compositor --bin compositor -- --socket waytransfer-0
WAYLAND_DISPLAY=waytransfer-0 weston-terminal
```

Run the second command in another Linux terminal with the same `XDG_RUNTIME_DIR`.
Additional options: `--listen 127.0.0.1:7878`, `--socket waytransfer-0`, `--help`.
Direct TCP is unauthenticated and unencrypted; use the SSH tunnel for remote use.

## Window sizing

Choose an initial size on the OMEN:

```sh
cargo run --release --features compositor --bin compositor -- --size 1024x768 -- weston-terminal
```

The Mac window starts at the first frame's advertised size. Drag its border or use
its zoom control to request a new remote size. Resize requests wait for 150 ms of
stable window size to avoid a full refresh on every drag event. While waiting, the
previous image stretches to fit; pointer coordinates map to the currently decoded
frame. The Linux compositor updates its virtual output and sends an xdg-shell
configure to the application. A new-size canvas may appear briefly before the app
commits its resized buffer. Applications can impose their own size hints.

Limits: positive dimensions, at most 2048 per side and 2,097,152 total pixels
(1920×1080 fits). Larger Mac windows request a proportionally fitted framebuffer
and display it stretched; this is a resolution budget, not HiDPI negotiation.
Frames and reset allocations are bounded. The binary packet ceiling is 12 MiB to
accommodate incompressible full frames within those limits; the input channel
remains capped at 4 KiB per packet. Normal compressed UI updates are far smaller.
Large/high-entropy redraws still cost bandwidth; use a smaller window on slow Wi-Fi.

## Current support and limits

- One xdg-shell toplevel, default 640×400, with configurable/resizable dimensions,
  scale 1. Extra toplevels receive a close request. Dialogs requiring another toplevel are therefore unsupported.
- Smithay handles surface commits, SHM buffers and subsurface state. Pixman renders
  the surface tree; popup positioning/rendering/grabs are implemented but have not
  yet received the same integration coverage as the main window.
- Pointer motion, three mouse buttons, scrolling and basic physical keyboard input.
  US layout only; no IME/text composition, layout negotiation or macOS shortcut
  translation. OS-reserved shortcuts may stay on the Mac. Custom remote cursor
  images are not rendered; the viewer uses its local pointer.
- No DMA-BUF/EGL/OpenGL clients, XWayland/X11, existing-desktop capture, window
  HiDPI, multiwindow management or clipboard exchange with macOS.
  GPU applications such as Kitty and X11 applications such as some IDE builds need
  further backend work. Start testing with `weston-terminal`.
- Linux content rendering and Mac presentation are working; application font/vector
  draw commands are not forwarded and there is no Metal renderer yet.

This is real Wayland application hosting with a deliberately limited scope, not a
replacement for a production desktop compositor. It creates its own session and
does not take over the OMEN's existing desktop.

## Bandwidth and responsiveness

Variable-size frames are split into 32×32 tiles, including partial edge tiles. Exact
comparison skips unchanged tiles; changed tiles use lossless LZ4 compression.
Idle content sends zero frame bytes. There is no persistent tile deduplication yet.
The compact, length-prefixed [binary protocol](PROTOCOL.md) sends compressed
tile bytes directly, avoiding the decimal arrays used by earlier JSON versions.

The compositor renders on surface commits at up to 30 ticks/second. A network
worker holds the latest raw framebuffer snapshot and encodes only the snapshots
it sends. It can skip intermediate raw frames without dropping encoded deltas or
breaking sequence numbers. Bounded input queues and separate network workers keep
slow writes out of the Wayland/UI loops. Input transitions are retained; excessive
viewer backlog closes the connection with an error instead of silently losing keys.

Each reconnect resets the encoder and starts with a complete frame at the latest
size. Every dimension change also sends a complete reset frame; partial deltas are
accepted only for the current size, and malformed resets leave the old frame intact. Packet lengths,
tile coordinates, sequence numbers and decompressed sizes are checked before an
update changes the client framebuffer. Sender logs actual message bytes versus
the current raw frame size (1,024,000 bytes at the default 640×400). RTT still limits
input response; these mechanisms reduce
bandwidth, not the network's round-trip delay.

## Reproduce bandwidth measurements

```sh
cargo run --release --example bandwidth
```

This compares identical LZ4 tile blocks wrapped in the exact compact v3 JSON
layout and in v4 binary packets. Includes the four-byte packet length; excludes
TCP/IP/SSH overhead. Measured on Apple Silicon with Rust 1.95:

| Synthetic workload | v3 JSON bytes | v4 binary bytes | Reduction |
| --- | ---: | ---: | ---: |
| 640×400 initial frame | 41,808 | 12,247 | 70.7% |
| 640×400 one-pixel pointer movement | 2,248 | 795 | 64.6% |
| 1920×1080 initial frame | 283,138 | 80,363 | 71.6% |
| 1920×1080 deterministic noise | 29,809,231 | 8,343,080 | 72.0% |

Idle content remains zero frame bytes. These isolate encoding overhead; they are
not measurements of real IDE workloads, wireless latency or network throughput.
Noise remains slightly larger than raw pixels with LZ4, so this format cannot make
arbitrary high-entropy content cheap to send. The test suite requires a >50%
reduction for the synthetic initial/small-delta cases and verifies exact pixels.

## Original square demo

Both commands can run on one Mac, as before:

```sh
cargo run --release --bin sender
cargo run --release --features viewer --bin viewer -- 127.0.0.1:7878
```

The GUI-free `viewer --headless` check is specific to the square demo, not the real
compositor:

```sh
cargo run --features viewer --bin viewer -- 127.0.0.1:7878 --headless
```

## Validation and contributions

```sh
cargo fmt --check
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo build --locked --all-features
```

Linux tests include an independent Wayland SHM client, pixel-exact reconstruction
across TCP, pointer clicks, keyboard press/release, frame callbacks, focus loss,
viewer disconnect/reconnect and application resize/repaint at odd dimensions.
Linux CI also starts the actual compositor binary and
`weston-terminal`, renders its pixels and executes remotely typed shell commands
before and after resizing:

```sh
WAYTRANSFER_TEST_WESTON=1 cargo test --locked --features compositor --test real_app -- --nocapture
```

macOS CI builds/tests the viewer and shared transport. Linux viewer builds additionally
need X11/Wayland development libraries listed in CI. A Mac-to-physical-OMEN GUI test
is still required; headless CI cannot verify native window appearance or the local
keyboard mapping on your hardware.

See [VALIDATION.md](VALIDATION.md) and [ROADMAP.md](ROADMAP.md). Contributions are
welcome; report workload, dimensions, bytes/update and network conditions with
bandwidth claims. MIT licensed; thanks to [Smithay](https://github.com/Smithay/smithay)
for the Wayland building blocks and [Pixman](https://www.pixman.org/) for software
composition.

For a manual native input regression (focus, click, A press/release and switching
away), see the `input_probe` instructions in [VALIDATION.md](VALIDATION.md).
If no address argument is supplied, the viewer also accepts `WAYTRANSFER_ADDRESS`;
the default remains `127.0.0.1:7878`.
