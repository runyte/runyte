# Native window paints the previous grid at new geometry

After a window resize or a font-size change, the native window (`--window`)
paints the last host frame at the new geometry until the host's frame for
the new grid arrives.

- **Font size** (`Ctrl-+`, `Ctrl-=`, `Ctrl--`): `NativeView::window_shortcut`
  replaces `self.metrics` and clears the glyph cache immediately. The next
  `render` computes the new grid size, sends `Event::Resize` to the host, and
  paints the existing frame (still laid out for the old grid) with the new
  cell metrics. For at least one frame the whole screen shows the old layout
  scaled to the new font: text overflows or leaves an empty band, and the
  cursor, media panes, pointer hit-testing and IME caret use cell positions
  that no longer match the drawn layout.
- **Window resize**: the old frame is painted at its old grid size in the
  new window. When the window grows, the uncovered area shows the root fill
  (see `native_window_margin_background.md`). When it shrinks, the grid is
  clipped, so the status and message lines at the bottom disappear until the
  next frame.

With GPUI's refresh-tick pacing (`native_window_frame_latency.md`) the stale
state usually lasts one or two display refreshes; it remains visible during
interactive resizing.

## Expected behavior

The window never paints a frame with cell metrics other than the ones it was
laid out for. After a font-size change, the previous frame keeps its previous
metrics until a frame for the new grid arrives. During resizing, a stale frame
keeps its own grid size with the theme background around it, and the frame for
the new size replaces it as soon as the host publishes it.

## Constraints

- No extra host round trip or timer is added; the window still paints the
  latest available frame.
- Pointer events, media hit-testing, the IME caret and the cursor use the
  metrics of the frame that is on screen, so a click lands on the cell the
  user sees.
- Frames from the host carry no metrics. The window knows which grid size it
  requested and can match an incoming frame's size against it.

## Reproduction

1. Run `target/release/runyte --window` and open a file.
2. Press `Ctrl-=` several times quickly, or drag the window edge.
3. Observe frames where the text is drawn at the new size but laid out for
   the old grid, or where the bottom lines are clipped.
