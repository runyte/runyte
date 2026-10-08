# Native window margins ignore the theme background

When the native window's size is not an exact multiple of the cell size, the
strips to the right of the last column and below the last row are painted
`#181818`, whatever the theme background is. With the default theme the cell
area is `#0b1f2a`, so a grey band appears along the right and bottom edges.
At 1910×1150 logical pixels with the default 15 px font (212×57 cells of
9×20 px), pixel readback showed a 2 px band on the right and a 10 px band at
the bottom.

The same grey shows while the window is being resized, between the moment the
window grows and the moment the host's frame at the new size arrives.

## Diagnosis

`src/native_frontend.rs` hard-codes the window colour:

- the root `div` in `NativeView::render` uses `.bg(rgb(0x181818))`;
- media layers use the same `rgb(0x181818)` fill;
- `cells.rs` resolves `Color::Reset` backgrounds to `0x181818` and
  `Color::Reset` foregrounds to `0xdddddd`.

The terminal frontend leaves padding and `Reset` colours to the terminal
emulator, whose default background normally matches the user's choice. The
window has no such default; the active theme's `background` and
`foreground` (`snapshot.editor.theme`) are the equivalent.

## Expected behavior

Margins, the area uncovered during a resize, media pane fills and `Reset`
cell colours use the active theme's background and foreground. Changing the
theme updates them with the next frame. Before the first frame arrives, the
window may keep a neutral fill.

## Constraints

- Applies in standalone windows and persistent-session attachments; both
  produce a `HostFrame` with the theme.
- Theme colours reach the window through `FrameData`, the same owned frame
  the cells come from, so the margin and the cells always come from one host
  frame.
- Reverse-video cells keep resolving `Reset` against the same defaults.

## Reproduction

1. Run `target/release/runyte --window` with the default theme.
2. Resize the window to a width that is not a multiple of 9 px or a height
   that is not a multiple of 20 px at the default font size.
3. Observe the grey strips at the right and bottom edges.
