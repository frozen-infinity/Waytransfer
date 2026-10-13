# Waytransfer

Open-source Rust software for using Linux applications from a MacBook over limited
bandwidth. This branch adds an **experimental headless, multiwindow Wayland
compositor** alongside the original square demo.

A Linux application runs in a separate Wayland session. Its buffers
are composed on Linux (software by default, optional EGL/GLES) and sent as lossless compressed tile deltas to
the Mac. The Mac displays the result in a native window and sends input back.

## Run a real application

Use stable Rust (1.95 or newer), and build the same branch on both machines.
**Wire protocol v5 requires rebuilding both sides; older v1/v2/v3/v4 viewers cannot
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

Type into the remote terminal, click, select text or scroll. Escape goes to
the application; closing a native window requests that application window to close. Switching away
from the viewer releases held keys/buttons. Reconnecting gets a fresh full frame.
The application continues while the viewer is disconnected. Ctrl-C in the Linux
compositor terminal ends the session and stops the directly launched application.
The compositor remains available when an application exits; Ctrl-C ends it.
Each Linux toplevel appears as a separate native Mac window. Focus, input, titles,
resizing and close requests are scoped to that window. Mac window dragging is local.

Choose another software-rendered Wayland application after `--`:

```sh
cargo run --release --features compositor --bin compositor -- -- your-wayland-app arg1
```

The software launcher sets `GSK_RENDERER=cairo`, `GDK_DEBUG=gl-disable`,
`QT_QUICK_BACKEND=software` and `QT_WAYLAND_DISABLE_WINDOWDECORATION=1`
for its child only. GPU mode omits the software-rendering overrides. These are helpful
software-backend defaults, **not a guarantee that every GTK/Qt application works**.
Avoid applications that forward launches to an already-running instance; they may
open on your existing desktop instead of this session.

For manual launches, start without a command:

```sh
cargo run --release --features compositor --bin compositor -- --socket waytransfer-0
WAYLAND_DISPLAY=waytransfer-0 weston-terminal
```

Run the second command in another Linux terminal with the same `XDG_RUNTIME_DIR`.
Repeat the second command to open more application windows.
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

## Optional GPU and X11 applications

Build these backends explicitly on Linux; the Mac viewer needs only `viewer`:

```sh
# Arch: optional dependencies (use your installed GPU driver)
sudo pacman -S --needed libglvnd mesa xorg-xwayland xterm
cargo run --release --features compositor,gpu,x11 --bin compositor -- --gpu --x11 -- weston-terminal
```

Debian/Ubuntu additionally needs `libegl1-mesa-dev libgles2-mesa-dev libwayland-dev
xwayland xterm`. Use `--gpu` for EGL/GLES composition and available EGL/DMA-BUF
imports. Use `--x11` for an isolated rootless XWayland server. These options can be
used independently. GPU initialization errors stop startup; software mode remains
available by omitting `--gpu`. Pixels are read back on Linux and sent as lossless
tiles; this is not forwarding GPU commands to the Mac.

The session prints its `WAYLAND_DISPLAY` and, with X11 enabled, its `DISPLAY`.
In another OMEN terminal, use those **printed values** and the same runtime directory:

```sh
WAYLAND_DISPLAY=waytransfer-0 weston-terminal
DISPLAY=:1 xterm # replace :1 with the printed DISPLAY
```

An app launched after `--` receives these values automatically. Avoid apps that
reuse an instance already running on your ordinary desktop. X11 needs a working
XWayland executable; GPU import depends on the installed EGL driver. Mesa software
EGL tests exercise the accelerated API path, not physical GPU performance or
NVIDIA compatibility. Kitty, IDEs and hardware DMA-BUF paths still need testing.

## JetBrains IDEs and application backend selection

New toplevels start as normal windows. Maximize/fullscreen and their reverse
requests receive configure responses; these states apply within the current remote
canvas. X11 focus also raises the selected X11 window so sibling windows cannot
intercept clicks at the shared remote origin.

The session implements `wp_viewporter` (buffer cropping/scaling) and
`wp_fractional_scale_v1` (preferred scale currently 1). JetBrains' native Wayland
runtime [requires viewporter during startup](https://github.com/JetBrains/JetBrainsRuntime/blob/44bb04f436c955532a14d7afd8b2387453c887c3/src/java.desktop/unix/native/libawt_wlawt/WLToolkit.c#L943).
This fixes the earlier `Can't bind to the wp_viewporter interface` startup failure;
rebuild the OMEN compositor to receive the change. The Mac viewer/protocol is unchanged.

Try CLion in the normal session (replace the executable path if needed):

```sh
cargo run --release --features compositor,gpu,x11 --bin compositor -- --gpu --x11 -- /opt/clion/bin/clion
```

`--app-backend auto` is the default: the child can use either session backend.
Use `--app-backend wayland` to select GTK/Qt Wayland and remove the child's X11
DISPLAY. A compatibility fallback is available for older or problematic toolkits:

```sh
cargo run --release --features compositor,gpu,x11 --bin compositor -- --gpu --x11 --app-backend x11 -- /opt/clion/bin/clion
```

X11 selection requires `--x11`. It sets GTK/Qt to X11, adds
`-Dawt.toolkit.name=XToolkit` to the child's `JDK_JAVA_OPTIONS`, and prevents a
Wayland connection to the existing desktop. It preserves other Java options.
App-specific VM options that explicitly force `WLToolkit` may override the Java
launch profile; remove the conflicting option or pass the IDE's documented
`-Dawt.toolkit.name=XToolkit` launch argument. These settings affect the launched
process and its children, not apps started separately in another terminal. Inherited
`WAYLAND_SOCKET` is removed; the isolated XWayland launch does not reuse the desktop's
`XAUTHORITY`.

CI runs Swing windows/dialogs with JetBrains Runtime 25.0.4+1-b508.27 on both
Wayland and X11, including keyboard text entry, resize and close. This tests the
runtime/toolkit path; it is not a full CLion project/plugin/driver compatibility
certification. Report the exact app version, launch command and stderr if an app
still fails. Instance forwarding can open an existing desktop instance instead.

## Current support and limits

- Viewport crop/scale and fractional-scale notification at scale 1; higher-resolution
  client buffers are supported without adding Mac HiDPI negotiation.
- Multiple Wayland xdg-shell and optional X11 windows, each with independent
  pixels, title, focus, size and lifecycle. Default initial size 640×400, scale 1.
  Limits: 16 windows and 8,388,608 total advertised pixels per session/viewer.
- Basic transient-parent relationships are passed to AppKit. Popup positioning,
  rendering and grabs exist; popups beyond the window canvas can be clipped.
  X11 override-redirect surfaces appear as separate native windows; desktop-grade
  menus, stacking and dialog behavior need further compatibility work.
- Pointer motion, three buttons, scrolling and basic US physical keyboard input.
  No IME, layout negotiation, macOS shortcut translation, remote cursor images,
  HiDPI negotiation or clipboard exchange. Mac OS shortcuts may stay on the Mac.
- Optional EGL/GLES offscreen rendering and DMA-BUF import advertisement when
  supported by the driver. Optional XWayland window management and keyboard focus.
- Application font/vector commands are not forwarded; there is no Metal renderer.

This hosts real applications in a separate experimental session. Whole-desktop
Hyprland streaming is a future separate mode; it does not capture or replace the
OMEN's current desktop. It is not a production desktop compositor.

## Bandwidth and responsiveness

Variable-size frames are split into 32×32 tiles, including partial edge tiles. Exact
comparison skips unchanged tiles; changed tiles use lossless LZ4 compression.
Idle content sends zero frame bytes. There is no persistent tile deduplication yet.
The compact, length-prefixed [binary protocol](PROTOCOL.md) sends compressed
tile bytes directly, avoiding the decimal arrays used by earlier JSON versions.

The compositor renders on surface commits at up to 30 ticks/second. A network
worker holds the latest raw scene snapshot and encodes only the snapshots
it sends. It can skip intermediate raw frames without dropping encoded deltas or
breaking sequence numbers. Bounded input queues and separate network workers keep
slow writes out of the Wayland/UI loops. Input transitions are retained; excessive
viewer backlog closes the connection with an error instead of silently losing keys.

Each window has an independent encoder and sequence. Each reconnect resets the encoders and starts with a complete frame at the latest
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
