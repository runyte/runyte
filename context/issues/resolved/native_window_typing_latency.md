---
title: "Native window typing appears late and in uneven bursts"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: 78ebbfb
---

## Resolution

`78ebbfb` (`Reduce native cell painting latency without changing glyph placement`)
replaces the expensive `paint_cells` path in `src/native_frontend.rs` with the
native-only painter in `src/native_frontend/cells.rs`. The old path allocated
and requested shaping for every visible symbol on every frame, painted a quad
for each background cell, and created a text layer for every symbol. The native
view also cloned the full cell buffer into each canvas closure.

The new view shares immutable frames through `Rc`, preserving the captured
frame identity, attachment checks, and acknowledgement bookkeeping. A bounded
cache retains cell-local layouts separately for the four existing font faces;
colour and decorations are applied during painting. Cell-local shaping retains
the original absence of cross-cell ligatures and places each symbol at its
Unicode-width grid position. Direct glyph submission preserves GPUI's emoji
path and the original baseline and decoration geometry.

Logical row layers preserve GPUI's draw ordering where rasterized italic glyphs
overhang adjacent rows. Directly submitting every glyph without logical layers
changed two blended pixels in the regression fixture. The cursor row and rows
with zero-width or oversized font advances retain exact per-cell layers, since
merging those bounds would alter overlap dependencies. Rows intersecting media
also retain per-cell backgrounds/layers so media scene depths and overlays do
not change. Ordinary backgrounds merge into horizontal colour runs, including
the root colour: retaining that ordering floor avoids changing the compositing
of glyphs between differently coloured rows. These are deliberate refinements
of the report's possible direction. GPUI can batch disjoint cell layers already;
per-cell layers did not categorically prevent batching as the initial diagnosis
suggested.

An opt-in `RUNYTE_NATIVE_PAINT_TIMING=1` trace measures CPU cell painting without
adding timers or wakeups. Three alternating release comparisons on Linux/X11
reduced the median of run medians from 13.560 ms to 0.933 ms at 120×40 cells.
The measured method, samples, binary hashes, and validation are retained in
`context/reference/native-window-experiment.md` and
`benchmarks/results/native-paint-2026-10-08.json`; the baseline timing patch is
`benchmarks/native-paint-before.patch`. This is scene-construction time, not a
measurement of display latency or a comparison with a terminal emulator.

Coverage includes
`backgrounds_merge_colors_and_preserve_media_overlay_occlusion` and
`glyph_cache_reuses_symbols_and_bounds_retained_text` in
`src/native_frontend/cells.rs`; the existing painted-input tests
`queued_native_input_keeps_its_painted_frame_and_cannot_confirm_unseen_plans` and
`ordinary_native_editing_remains_queued_across_unpainted_frames` in
`src/native_frontend/tests/input.rs`; and `tests/native_window.py` full X11
acceptance plus its `--paint-benchmark` and `--paint-styles --paint-reference`
modes. The style fixture compares 20 rows pixel-for-pixel, covering the four
font faces, modifiers, decorations, emoji, Nerd Font icons, combining text,
wide-cell advancement, alternating backgrounds, and a cursor beside an italic
icon. All repository checks passed; canonical default-feature line coverage was
92.05%, above the unchanged 89% floor. Astra High review findings were addressed
and the final source/harness review was clean.

Known limitation: macOS interaction and latency still need a separate manual
run. The local style fixture's CJK characters use missing-glyph boxes because
no CJK fallback font is installed, so it verifies their grid advancement but
not actual CJK glyph appearance. Media rows and unusual logical advances retain
per-cell layers for visual correctness; cached shaping still applies there.

## Report

### Observed behavior

In the experimental native window (`--window`, built with `--features native`),
typed letters appear on screen noticeably later than in the terminal frontend.
The delay is also uneven: some keystrokes show up promptly, while others lag
and then appear together with later ones. The same behavior occurs on Linux and
macOS, in both cases with a release build (`cargo build --release --features
native`). Running the same editor inside a terminal emulator such as Alacritty
does not show it.

Typing in a buffer is the main case. The integrated terminal panes, the
prompts and every other surface share the same painting path described below,
so they are affected the same way.

### Diagnosis

The keystroke path itself is short and is not paced:

1. The GPUI `on_key_down` listener in `NativeView::render`
   (`src/native_frontend.rs`) converts the keystroke with `translate_key` and
   sends it to the host thread over the bridge channel. GPUI 0.2.2 on macOS
   (`platform/mac/window.rs`, `handle_key_event`) delivers a printable key to
   this listener before the input method unless a composition is active, so
   input-method routing does not delay plain letters.
2. The host loop in `src/main.rs` applies the input and draws immediately.
   The 16 ms `FRAME_INTERVAL` tick paces only background work.
3. `Surface::draw` renders Ratatui into a `TestBackend`, clones the whole cell
   `Buffer` into `FrameData`, and wakes the GPUI thread.
4. GPUI re-renders the view on the next frame and paints the grid in
   `paint_cells`.

The cost is in step 4. `paint_cells` repaints the entire grid on every frame,
whatever changed, and does per-cell work that a terminal renderer normally
avoids:

- One `paint_quad` background per cell, including cells whose background
  matches the window's default `0x181818`, which the root `div` already paints.
- For every non-blank cell, a fresh `String`, a `Font` and a `TextRun`, then
  `window.text_system().shape_line(...)` and `ShapedLine::paint`. These go
  through GPUI's line-layout cache and glyph-atlas locks once per cell.
- `ShapedLine::paint` (GPUI `text_system/line.rs`, `paint_line`) wraps each
  call in `window.paint_layer(...)`. Every cell therefore becomes its own
  scene layer, which prevents the renderer from batching primitives across
  cells.

At the default 120×40 window that is about 4,800 background quads, up to 4,800
shaping calls and up to 4,800 layers per keystroke. A maximized window on a
high-density display can exceed 25,000 cells. When one paint takes longer than
a display refresh interval, the frames in between are coalesced, which accounts
for letters appearing in uneven bursts rather than after a constant delay.

`NativeView::render` also clones the full `FrameData` (`let frame =
self.frame.clone()`) on every render to move it into the `canvas` paint
closure, which copies the cell buffer once more per frame.

No timings have been recorded yet. The diagnosis comes from reading the code
path, not from a profile.

### Expected behavior

Typing in the native window is at least as responsive as typing in the
terminal frontend inside a GPU terminal emulator, without visible bursts.
Painting cost follows what is drawn, so per-frame work no longer grows with
one quad, one shaping call and one layer per cell.

### Constraints

- Rendering stays visually identical: the same font faces, weights and
  styles; `REVERSED`, `DIM`, `HIDDEN`, `UNDERLINED` and `CROSSED_OUT` as
  `paint_cells` applies them today; the same colour mapping in `color`; wide
  characters advancing by their `unicode-width`; the existing cursor bar.
- Glyphs stay on the cell grid. `CELL_WIDTH` (9.0) equals JetBrains Mono's
  0.6 em advance at the 15 px font size, but Nerd Font icons, fallback-font
  glyphs and wide characters do not necessarily advance by one cell. Shaping a
  multi-cell string must not drift them off the grid. Bundled JetBrains Mono
  has contextual-alternate ligatures. Today each cell is shaped on its own, so
  none appear, and a change must not introduce them unintentionally.
- Colour emoji keep being painted through the emoji path (`paint_emoji`), as
  `ShapedLine::paint` does for glyphs marked `is_emoji`.
- Cells under media panes stay unpainted unless an overlay covers them
  (`FrameData::under_media`). Images remain behind key hints and prompts.
- The painted-frame identity, `presented` bookkeeping, attachment checks and
  presentation acknowledgements in the `canvas` paint closure keep their
  current semantics. Approval admission depends on them.
- No GUI timer or polling loop is added. The bridge keeps only the latest
  frame and coalesces wakeups.
- The change stays within the optional `native` feature, and the default build
  is unaffected.

### Possible direction

The following summarizes the GPUI 0.2.2 public API as checked; it does not
prescribe the fix.

- `GlyphId` cannot be constructed outside GPUI, but
  `WindowTextSystem::layout_line` returns an `Arc<LineLayout>` whose public
  `runs[].font_id` and `runs[].glyphs[]` (`id`, `position`, `is_emoji`) carry
  shaped glyph ids, along with the line's `ascent` and `descent`.
  `Window::paint_glyph`, `Window::paint_emoji`, `Window::paint_underline` and
  `Window::paint_strikethrough` are public. A cache owned by the view, keyed
  by cell symbol plus bold and italic, can therefore shape each distinct
  symbol once and paint glyphs at exact grid positions without shaping on
  every frame and without one layer per cell. This is close to how terminal
  emulators use a glyph atlas.
- To match the current output, the baseline and decoration offsets follow
  `paint_line`: baseline at `(CELL_HEIGHT - ascent - descent) / 2 + ascent`,
  underline at `baseline + descent * 0.618`, strikethrough at
  `(ascent * 0.5 + baseline) * 0.5`.
- Backgrounds can be merged into one quad per horizontal run of identical
  colour, stopping at media-covered cells, and skipped where they equal the
  default background.
- Holding the latest frame behind `Rc` (or `Arc`) in `NativeView` removes the
  per-render buffer copy.
- An opt-in timing trace would let latency be measured on each platform:
  paint duration per frame, and the time from key-down to the first painted
  frame newer than the one presented when the key was captured.

### Reproduction

1. `cargo build --release --features native`
2. Run `target/release/runyte --window` in any workspace and open a source
   file.
3. Type a sentence at normal speed in Insert mode. Letters appear late and
   unevenly.
4. Maximize the window and repeat. If per-cell painting is the cause, the lag
   grows with the window's cell count.
5. For comparison, run `target/release/runyte` inside a GPU terminal emulator
   and type the same text.

### Validation

Besides the ordinary repository checks and the native adapter checks listed in
`context/reference/native-window-experiment.md`, run the isolated X11
acceptance in `tests/native_window.py`. It confirms that first paint, modal
editing, hints, splits, media panes and integrated PTY input still render and
respond. Record before-and-after paint timings for the same window size in the
"Measured acceptance" section of that reference. macOS needs a separate manual
run.
