---
title: "Native window cell painting cost grows with the grid"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: 67e0211
---

## Resolution

`67e0211` (Reuse unchanged native cell styles and painted glyph rows).

`paint_cells` and `backgrounds` repeated colour conversion, media checks, glyph
hashing and width calculation across the entire grid. Prepared rows now retain
resolved foreground/background runs, logical widths and coverage while the
immutable source row and its presentation inputs remain unchanged. ASCII glyphs
have direct-indexed slots, with the per-face cache still bounded to 1,024 layouts.
Theme changes rebuild colours without discarding shaped layouts.

The vendored GPUI now exposes draw-only previous-scene replay through
`Window::paint_cached_scene`. Unchanged rows reuse their glyph primitives rather
than repeating glyph raster-bound and atlas lookups. Only immediately adjacent
frames with matching window, scale, clip and opacity can reuse a segment;
Runyte additionally checks content, colours, media coverage, font metrics and
cursor-row status. The handle holds scene indices, not another scene or texture.
Backgrounds remain below glyphs; row and oversized-glyph layer ordering is
preserved. Input handlers and presentation acknowledgements run on every frame.

`tests/native_window.py --paint-styles --paint-reference` matched all styled-row
pixels against the pre-change release. Repeated isolated X11/lavapipe release
benchmarks reduced median scene construction from 881 to 402 µs at 120×40,
1,591.5 to 882 µs at 213×70, and 1,773.5 to 959 µs at 266×88. The harness now
accepts `--paint-font-size` and `--paint-window`. Raw samples, hashes, environment
and timing variability are recorded in
`benchmarks/results/2026-10-08-native-cell-paint.json` and
`context/reference/native-window-experiment.md`.

`unchanged_rows_reuse_prepared_styles_and_changed_rows_do_not`,
`prepared_rows_refresh_theme_and_media_coverage_without_changed_cells`, and
`prepared_wide_and_hidden_cells_preserve_logical_advance_and_styles` in
`src/native_frontend/tests/cells.rs` cover row invalidation, media/overlay
coverage and styled logical advance. `glyph_cache_reuses_symbols_and_bounds_retained_text`
in `src/native_frontend/cells.rs` covers ASCII reuse and bounded non-ASCII storage.
The native frontend tests and native all-target Clippy pass.

The debug-window acceptance additionally caught a paint-phase call to GPUI’s
prepaint-only `element_offset` getter. The replay helper now relies on the
caller’s absolute placement check instead; it does not read prepaint state.

Known limitation: these timings measure CPU scene construction on software
Vulkan, not hardware-GPU rendering or end-to-end latency. Wayland desktop and
macOS runtime validation remain outstanding.

## Report

The native cell painter (`src/native_frontend/cells.rs`) repaints the whole
grid on every GPUI frame. Its CPU cost grows with the number of cells, and
several per-cell steps repeat work whose result is the same for every frame.

## Measurements

`RUNYTE_NATIVE_PAINT_TIMING=1`, release build, typing in Insert mode
(Linux, Radeon 880M, rootful Xwayland at 60 Hz):

| Grid | Cells | Median paint | p90 paint |
| --- | ---: | ---: | ---: |
| 120×40 | 4,800 | 0.37 ms | 0.56 ms |
| 212×57 | 12,084 | 0.68 ms | 1.09 ms |
| 318×86 | 27,348 | 1.1–1.9 ms | 1.4–2.2 ms |

This is scene construction only. GPUI also lays out the element tree and the
GPU renders the scene on every frame.

## Diagnosis

Per cell and per frame, `paint_cells` and `backgrounds`:

- convert each `ratatui` colour through `color()` into `rgb(..).into()`, an
  RGB-to-HSL conversion, once for the background and once for the foreground;
  `background()` is evaluated again for every comparison while merging runs;
- call `FrameData::under_media`, which scans every media pane and every
  overlay rectangle for each cell, and call `media_row` for each step of the
  background run loop;
- look up the glyph layout in a `HashMap<String, Arc<LineLayout>>` keyed by
  the cell symbol, hashing the symbol with the default SipHash hasher, even
  for ASCII text;
- compute `unicode_width` for each symbol.

## Expected behavior

Per-frame painting cost for an ordinary text grid is reduced substantially at
every grid size, without changing a single painted pixel. Typical targets:
resolving colours and media coverage once per frame, row or run rather than
per cell, and a direct-indexed fast path for single-byte ASCII glyph layouts.

## Constraints

- Output stays pixel-identical; `tests/native_window.py --paint-styles
  --paint-reference` compares styled rows against a previous build.
- The glyph cache stays bounded per face, and theme changes do not invalidate
  shaped layouts.
- Media occlusion, overlays above media, the cursor-row and oversized-glyph
  layer rules in `cells.rs` keep their current ordering.
- Before/after timings use `RUNYTE_NATIVE_PAINT_TIMING=1` and
  `tests/native_window.py --paint-benchmark`, as recorded in
  `context/reference/native-window-experiment.md`.

## Reproduction

```sh
cargo build --release --features native
RUNYTE_NATIVE_PAINT_TIMING=1 DISPLAY=:94 python3 tests/native_window.py \
  --binary target/release/runyte --paint-benchmark --output <directory>
```

Repeat with a larger window or a smaller `editor.font_size` to see the cost
grow with the cell count.
