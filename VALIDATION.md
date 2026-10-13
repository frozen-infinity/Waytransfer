# Validation

## Original transport slice

Apple Silicon macOS, Rust 1.95.0: formatting, 7 integration tests, all-feature build,
Clippy with warnings denied and separate-process demo smoke check passed.
The synthetic initial frame was 41,761 bytes and the center-to-(40,40) pressed
pointer update was 2,695 bytes versus 1,024,000 raw bytes. These are demo-only
measurements including JSON/framing, excluding TCP/SSH headers.

## Single-window compositor

The Linux-only backend is excluded from macOS compilation. macOS validation covers
the revised viewer, key mapping and shared transport, including reconnect, frame
coalescing and shutdown. Linux CI builds and checks the actual Smithay/Pixman backend.

The independent Wayland-client test creates an xdg-shell window, submits ARGB8888
SHM buffers and compares reconstructed TCP pixels exactly, including distinct
corners to detect flipped images/channel mistakes. It verifies pointer clicks,
keyboard enter/press/release, frame callbacks and held-key release after focus loss
and viewer disconnect/reconnect.

The real-application test starts the compositor binary in a private runtime
directory, launches weston-terminal, receives nonuniform application pixels and
sends evdev key snapshots for `touch keyboard-ok`. A created file proves that the
terminal executed the remote command. It is enabled explicitly with
`WAYTRANSFER_TEST_WESTON=1`, and mandatory in Linux CI. See the PR checks for the
results of the final revision; an unset variable skips this optional local test.

Popup/subsurface compatibility beyond the tested primary surface, GUI rendering
on physical Mac/OMEN hardware, IDEs, GPU clients, wireless bandwidth and latency
have not been validated. No production-compositor or broad application-compatibility
claim follows from these tests.

## macOS input focus correction

The macOS implementation of minifb 0.29.0 returns `mfb_is_active(...) == 0`, while
its native function returns a boolean active flag. Its notification tracker also
sets the flag to true when the same window resigns main status. The viewer's input
gate therefore cannot use that API, or merely invert it for reliable focus loss.

Waytransfer now queries AppKit's live `NSApplication.isActive` and
`NSWindow.isKeyWindow` state through a small macOS-only helper. Other platforms
continue using minifb's focus API. No wire-protocol or compositor change is needed.

Manual native regression on Apple Silicon macOS used the actual viewer executable
and the `input_probe` receiver over TCP. Observed focus gain, pointer click, A key
press/release and focus loss after switching to another application; probe passed.
Unlike the earlier headless tests, this exercises the real Mac input gate.

Repeat manually in two terminals:

```sh
cargo run --example input_probe
cargo run --features viewer --bin viewer -- 127.0.0.1:17879
```

Click inside the viewer, press and release A, then switch to another application.
The probe reports PASS when it receives all expected events; it times out after
five minutes. This is a manual GUI check, not automatically run by headless CI.

## Resizable one-window mode (protocol v3)

Shared tests cover grow/shrink and equal-area shape changes, exact reconstruction,
partial edge tiles, malformed reset atomicity, size/pixel limits, 150 ms debounce,
stretched-window pointer mapping and reconnection after coalesced size changes.
An incompressible 1920×1080 round trip verifies that a legal full frame fits the
40 MiB JSON packet ceiling. Input packets retain the separate 4 KiB limit.

Linux Wayland tests resize to 801×503 and 351×227, wait for the client's actual
resized pixels (not just the compositor's new canvas), and verify pointer clicks
beyond the original 640×400 area. The real weston-terminal test starts with an
803×509 canvas, resizes to 961×607 and executes remote commands before/after resize.
See the PR's CI results for final Linux/macOS verification.

Manual Apple Silicon native-window check passed: the actual viewer's zoom control
requested 1710×967, a new-size frame was delivered to the viewer, and A press/release reached
the sender afterward. Repeat with `cargo run --example input_probe -- --resize`,
connect the viewer to 127.0.0.1:17879, resize/zoom, then press and release A.
The physical OMEN/Mac application resize still requires the user's hardware check.

## Binary protocol (v4)

The actual network path now uses a compact binary codec and raw LZ4 bytes, removing
direct serde/serde_json dependencies. The packet ceiling shrinks from 40 MiB to
12 MiB; input packets keep their 4 KiB limit. Existing lossless/reset/resize/input
and reconnect tests exercise this same binary path.

New tests pin independently written byte fixtures for all control messages and a
one-pixel LZ4 frame; test concatenation, one-byte-at-a-time reads, truncation,
unknown tags, noncanonical booleans, invalid dimensions/counts/keys/tile lengths,
trailing bytes and invalid outgoing messages. A deterministic 8,192-packet malformed
corpus must not panic. This is regression coverage, not a full fuzzing campaign.

`cargo run --release --example bandwidth` compares the same compressed tile payloads
using an exact v3 compact JSON reference envelope and the production v4 writer.
Initial 640×400 frame: 41,808 → 12,247 bytes (70.7%); one-pixel movement:
2,248 → 795 bytes (64.6%). Initial 1920×1080 frame: 283,138 → 80,363 bytes
(71.6%); deterministic noise: 29,809,231 → 8,343,080 bytes (72.0%). Includes framing,
excludes TCP/IP/SSH headers. Pixels remain exact. See README for the comparison
method; this does not imply a corresponding latency or wireless-throughput gain.

Local Apple Silicon build/tests/formatting/Clippy passed. Linux/macOS CI runs the
same checks, including real Wayland SHM and weston-terminal input/resize tests.
Consult the PR checks for the final revision. Native GUI focus/resize checks above
were performed before the v4 codec change; the UI/event-handling code is unchanged.
Physical OMEN/Mac v4 testing remains a hardware check.
