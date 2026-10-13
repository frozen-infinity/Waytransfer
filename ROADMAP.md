# Roadmap

Delivered:

- Synthetic remote-display transport, dirty tiles and lossless LZ4.
- Experimental headless Linux compositor: one xdg-shell application window,
  wl_shm software buffers, fixed 640×400 virtual output and surface composition.
- Native Mac viewer with pointer/buttons/scroll and basic US physical keyboard.
- Background transport, bounded queues, raw-frame coalescing and full reconnect.
- Shared transport tests, independent Wayland-client integration test and a real
  weston-terminal smoke test in Linux CI.

Next:

1. **One-window usability:** configurable/negotiated dimensions, resize/reset,
   window title and lifecycle metadata, HiDPI, remote cursor shapes, keyboard layout
   selection and IME. Expand popup/subsurface tests and test common GTK/Qt apps.
2. **GPU and legacy applications:** EGL/DMA-BUF capture/rendering path and XWayland
   support. Benchmark software and GPU paths with terminals and IDEs before claims.
3. **Bandwidth:** binary wire format, scroll/copy detection, bounded tile cache,
   configurable refresh budget, realistic editor workloads, input-to-display latency
   under constrained bandwidth and RTT. Fuzz decoders and bound Wayland resources.
4. **Multiwindow:** native Mac windows per Linux toplevel, transient dialogs,
   independent framebuffers, focus/stacking and clipboard. Keep dragging and Mac
   window decorations local. Do this after single-window compatibility is reliable.
5. **Rendering on Mac:** measured GPU presentation and optional semantic/vector
   operations for cooperating applications/toolkits. Ordinary Wayland clients submit
   rendered buffers; Wayland forwarding alone cannot expose their drawing commands.
6. **Harden/release:** authenticated encrypted transport, session authorization,
   client/resource limits, reconnect lifecycle, process supervision, packaging,
   compatibility matrix and release artifacts.

Current code hosts applications in a separate session. It does not capture the
existing desktop, support a whole desktop environment, or replace the Linux display
manager. Multiwindow and accelerated-client support are not delivered yet.
