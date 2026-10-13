# Roadmap

Delivered (experimental):

- Rust lossless tile/LZ4 transport, idle suppression, compact binary framing,
  bounded queues, latest-scene coalescing and complete reconnect/reset frames.
- Headless Smithay session with multiple Wayland toplevels, independent native Mac
  windows, titles, focus, close, debounced resizing and basic transient parents.
- Software Pixman composition, optional EGL/GLES composition and driver-supported
  DMA-BUF import; optional rootless XWayland with per-window input/focus.
- Viewport crop/scale and scale-1 fractional notifications; explicit GTK/Qt/Java
  X11 launch profile and JetBrains Runtime native/X11 regression fixtures.
- Shared transport tests, independent Wayland client tests and actual application
  smoke tests. See VALIDATION.md for verified paths and outstanding hardware checks.

Next:

1. **Compatibility:** physical GPU/driver matrix (including NVIDIA), Kitty and IDEs,
   GPU-backed X11/GLX, GTK/Qt dialogs, popup/subsurface geometry, X11 stacking and
   override-redirect behavior. Reconnect tests with real apps and many windows.
2. **Window usability:** HiDPI, remote cursors, clipboard, keyboard layouts and IME;
   native session controls and clean process supervision.
3. **Bandwidth:** scroll/copy detection, bounded tile cache, configurable refresh
   budget, realistic editor workloads, measured input latency on constrained links.
   Fuzz decoders and bound client-side Wayland resources.
4. **Whole Hyprland mode:** a separate desktop/output stream, initially from a
   nested Hyprland session or a supported capture/input integration. Preserve the
   full desktop, workspace animations, panels and compositor shortcuts in one Mac
   viewport. Choose and validate the capture/control APIs before implementation;
   the per-application session here does not host Hyprland or capture an existing one.
5. **Rendering on Mac:** measured GPU presentation and optional semantic/vector
   operations for cooperating toolkits. Ordinary Wayland clients submit rendered
   buffers; forwarding Wayland cannot expose their drawing commands.
6. **Release:** authenticated encrypted transport, authorization, resource hardening,
   packaging, compatibility matrix and release artifacts.
