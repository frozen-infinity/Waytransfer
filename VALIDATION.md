# Initial slice validation

Local validation on Apple Silicon macOS with Rust 1.95.0:

- `cargo fmt --check`: passed.
- `cargo test --locked --all-features`: passed (7 integration tests).
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: passed.
- `cargo build --locked --all-features`: passed, including native viewer.
- Separate sender and `viewer --headless` processes over loopback: passed handshake,
  initial reconstruction, pointer input and exact delta reconstruction.

The process smoke test measured 41,761 bytes for the initial synthetic frame and
2,695 bytes for the update from the center to (40,40), with left button pressed.
Raw frame size is 1,024,000 bytes. Includes JSON/framing overhead, excludes TCP/SSH
headers. This is not a real-app, Wi-Fi, latency or compositor benchmark.

GUI visual/interactive behavior and communication between a physical Linux OMEN
and MacBook have not been manually verified. Linux compilation/testing is covered
by the CI matrix; consult the PR checks for their actual status.
