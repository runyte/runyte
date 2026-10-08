# Native window cell painting cost grows with the grid

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
