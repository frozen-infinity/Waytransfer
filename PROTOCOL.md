# Waytransfer wire protocol v4

Experimental single-surface protocol over one bidirectional, ordered TCP stream.
Both peers send Hello first and require version 4. Versions 1–3 use JSON and are
incompatible; rebuild both machines. There is no automatic downgrade.

Each packet begins with a **u32 big-endian body length**, excluding the four-byte
prefix. Zero and lengths above **12 MiB** are rejected before allocating a body.
The server input channel applies a smaller **4 KiB** limit. A body starts with a
one-byte tag. Fields are tightly packed, without padding. Multi-byte numeric fields
are big-endian; the compressed pixel payload uses little-endian u32 pixels.
Booleans are exactly 0 or 1. Unknown tags, malformed fields and trailing bytes are
errors; do not skip unknown messages in this version.

| Tag | Message | Body fields after tag |
| --- | --- | --- |
| 1 | Hello | version: u16 |
| 2 | Frame | sequence: u64, width: u16, height: u16, reset: u8, tile count: u16, tiles |
| 3 | Pointer | x: u16, y: u16, left pressed: u8 |
| 4 | Resize | width: u16, height: u16 |
| 5 | Keys | count: u8, held evdev codes: count × u8 |
| 6 | Buttons | right pressed: u8, middle pressed: u8 |
| 7 | Scroll | horizontal: i16, vertical: i16 |
| 8 | Focus | active: u8 |

A Frame has a 16-byte fixed body header including its tag. Each tile contains
`x: u16, y: u16, compressed length: u16, LZ4 bytes`. Tile coordinates are pixel
origins, aligned to 32. Tile width/height are min(32, remaining framebuffer extent).
The payload is a raw LZ4 block without a prepended size. Expected decompressed
length is derived from the tile rectangle: width × height × 4. Pixels are packed
little-endian u32 values in row-major order. Captured content uses 0x00RRGGBB for
minifb presentation; the codec preserves all 32 bits.

The codec checks positive dimensions, max 2048 per side, max 2,097,152 pixels,
nonzero sequence, nonempty/bounded tile count, tile alignment/extents, nonzero
compressed lengths and the LZ4 encoder's maximum block size for that tile before
copying its data. Parsing validates the tile count against both frame dimensions
and available body bytes before reserving the tile vector.

The framebuffer decoder additionally checks sequence order, duplicate tiles,
actual LZ4 output length and complete coverage for reset frames. An initial frame
must reset. Dimension changes require reset; a partial delta cannot change size.
All tiles are validated/reconstructed in a temporary framebuffer before committing
the pixels, dimensions or sequence. A new connection starts sequence at 1 and sends
a complete current-size frame. Within one connection, sequences increase across
resize resets. Unchanged content sends no frame packet.

Pointer coordinates must be below 2048; the compositor clamps in-flight input to
its current dimensions during resize transitions. Keys accepts at most 128 codes,
each 1–127; duplicates have set semantics in the compositor. Keys and Buttons are
held-state snapshots. Scroll values are signed deltas. Focus loss/disconnect releases
held input. Resize is a request, not an acknowledgement: the subsequent Frame size
reports the current canvas, which may precede an application's resized buffer commit.

Client-to-server after Hello: Pointer, Resize, Keys, Buttons, Scroll and Focus.
Server-to-client after Hello: Frame only. Authentication/encryption is supplied by
the recommended SSH tunnel, not this protocol.

Golden fixtures and invalid/fragmented packet tests are in `tests/wire.rs`. For
example, Focus(false) is exactly:

```text
00 00 00 02 08 00
```

This format removes JSON decimal-array overhead. It adds no lossy compression or
new tile cache; those are separate decisions. Stream boundaries are defined by
length prefixes, never by individual TCP reads or writes.
