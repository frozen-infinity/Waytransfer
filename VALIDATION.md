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
