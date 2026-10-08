---
title: "Native window margins ignore the theme background"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: 707c732
---

## Resolution

`707c732` (`Paint native window margins and default colours from the theme`)
carries the theme's default colours in every `FrameData`, beside the cells
they belong to. `src/native_frontend.rs` previously hard-coded `#181818` for the
root `div` and media layers and `#cccccc` for media labels, and `cells.rs`
resolved `Color::Reset` to `#181818`/`#dddddd`, so every margin, the area
uncovered during a resize and any `Reset` cell ignored the theme.

`capture_snapshot_media`, which both the standalone draw closures and
persistent-session attachments (`capture_attached`) call for the snapshot
being rendered, now records `runyte::ui::native_default_colors` of
`snapshot.editor.theme`, and `Surface::draw` copies them into the frame it
publishes. The root fill, media fills, media labels and `Reset` resolution in
the cell painter read them from the frame on screen, so a margin and its cells
can never come from different themes. `color()` was split so `rgb_value`
resolves a cell colour to RGB with a caller-supplied default.

A theme may declare `background: reset` or `foreground: reset`, meaning the
terminal emulator's own colour. The window has no such default, so
`native_default_colors` picks one that contrasts with the colour the theme does
define, using the luminance at which black and white text contrast equally
(about 0.179): a light `#f8f8f8` window with `#202020` text, or a dark `#181818`
window with `#dddddd` text. The previous hard-coded colours matched only dark
themes. Terminal panes are unaffected in practice: the renderer already maps a
terminal's default colours onto the theme's foreground and pane background, so
their cells are `Reset` only when the theme colour itself is `reset`, where the
terminal frontend likewise leaves them to the emulator's defaults. Before the
first frame arrives the window keeps the `#181818` fallback.

Tests: `reset_colours_follow_the_frame_theme` in
`src/native_frontend/cells.rs` covers `Reset` backgrounds and reverse video
against frame colours; `native_default_colors_contrast_with_the_defined_theme_colour`
in `src/ui.rs` covers defined, `reset` and mid-grey theme colours. The
standalone run of `tests/native_window.py` resizes the window to 1085×805, which
leaves a 5 px margin at the default 9×20 px cells, and requires the right and
bottom margin pixels to equal an empty editor cell; the build before this
change fails it with `#181818` against `#0b1f2a`.

Known limitation: a stale frame is still shown at its own grid size during a
resize until the host's frame for the new size arrives; that is tracked by
`native_window_stale_grid_geometry.md`.

## Report

When the native window's size was not an exact multiple of the cell size, the
strips to the right of the last column and below the last row were painted
`#181818`, whatever the theme background was. With the default theme the cell
area is `#0b1f2a`, so a grey band appeared along the right and bottom edges.
At 1910×1150 logical pixels with the default 15 px font (212×57 cells of
9×20 px), pixel readback showed a 2 px band on the right and a 10 px band at
the bottom.

The same grey showed while the window was being resized, between the moment
the window grew and the moment the host's frame at the new size arrived.

### Diagnosis

`src/native_frontend.rs` hard-coded the window colour:

- the root `div` in `NativeView::render` used `.bg(rgb(0x181818))`;
- media layers used the same `rgb(0x181818)` fill;
- `cells.rs` resolved `Color::Reset` backgrounds to `0x181818` and
  `Color::Reset` foregrounds to `0xdddddd`.

The terminal frontend leaves padding and `Reset` colours to the terminal
emulator, whose default background normally matches the user's choice. The
window has no such default; the active theme's `background` and `foreground`
(`snapshot.editor.theme`) are the equivalent.

### Expected behavior

Margins, the area uncovered during a resize, media pane fills and `Reset` cell
colours use the active theme's background and foreground. Changing the theme
updates them with the next frame. Before the first frame arrives, the window
may keep a neutral fill.

### Constraints

- Applies in standalone windows and persistent-session attachments; both
  produce a `HostFrame` with the theme.
- Theme colours reach the window through `FrameData`, the same owned frame the
  cells come from, so the margin and the cells always come from one host frame.
- Reverse-video cells keep resolving `Reset` against the same defaults.

### Reproduction

1. Run `target/release/runyte --window` with the default theme.
2. Resize the window to a width that is not a multiple of 9 px or a height
   that is not a multiple of 20 px at the default font size.
3. Observe the grey strips at the right and bottom edges.
