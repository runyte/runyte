# Native window typing latency

## Observed behavior

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

## Diagnosis

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

## Expected behavior

Typing in the native window is at least as responsive as typing in the
terminal frontend inside a GPU terminal emulator, without visible bursts.
Painting cost follows what is drawn, so per-frame work no longer grows with
one quad, one shaping call and one layer per cell.

## Constraints

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

## Possible direction

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

## Reproduction

1. `cargo build --release --features native`
2. Run `target/release/runyte --window` in any workspace and open a source
   file.
3. Type a sentence at normal speed in Insert mode. Letters appear late and
   unevenly.
4. Maximize the window and repeat. If per-cell painting is the cause, the lag
   grows with the window's cell count.
5. For comparison, run `target/release/runyte` inside a GPU terminal emulator
   and type the same text.

## Validation

Besides the ordinary repository checks and the native adapter checks listed in
`context/reference/native-window-experiment.md`, run the isolated X11
acceptance in `tests/native_window.py`. It confirms that first paint, modal
editing, hints, splits, media panes and integrated PTY input still render and
respond. Record before-and-after paint timings for the same window size in the
"Measured acceptance" section of that reference. macOS needs a separate manual
run.
