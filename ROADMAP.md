# Roadmap

1. **Transport prototype (current):** synthetic single surface, dirty tiles, LZ4,
   retained client framebuffer, pointer round trip, Linux/macOS CI.
2. **Measure:** code-editor-like workloads, bytes/update and input-to-display
   latency under constrained bandwidth/RTT; binary wire format, coalescing and
   explicit backpressure. Add decoder fuzzing, limits on sessions and timeouts.
3. **Real Linux pixels:** choose and implement a Wayland capture backend with
   explicit user consent; negotiate surface size, resize/reset and cursor metadata.
   Capture is distinct from hosting applications in a compositor.
4. **Application windows:** evaluate a nested compositor (e.g. Smithay), surface
   lifecycle, damage, subsurfaces, input seats, keyboard layouts and clipboard.
   Wayland clients generally submit rendered buffers, not font/vector draw commands.
   Forwarding Wayland alone therefore does not make the Mac render application UI.
5. **Bandwidth:** bounded content-addressed tile cache, scroll/copy detection,
   adaptive update rates and per-surface prioritization, benchmarks before claims.
6. **macOS integration:** native multiwindow shell, local decorations, HiDPI,
   accessibility and a measured GPU presentation path. Explore semantic/vector
   rendering only for applications/toolkits that expose suitable operations.
7. **Harden:** authenticated encrypted transport, reconnect/resync, resource budgets,
   multi-client policy, packaging, compatibility matrix and release process.

No real Wayland compositor support is delivered or implied by milestone 1.
