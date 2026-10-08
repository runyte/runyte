---
title: "Native window copies the whole cell grid for every host frame"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: de7405c
---

## Resolution

Commit `de7405c` (`Share unchanged native cell rows between immutable frames`)
replaces the full-buffer clone in `Surface::draw` with `GridBackend` in
`src/native_frontend/grid.rs`. Ratatui already identifies changed cells; the
backend now applies them directly to copy-on-write `Arc<[Cell]>` rows. Publishing
clones row references rather than every cell. Multiple changes to one row copy
it at most once while older frames still own it. The GUI receives a complete
immutable `Grid`, including changes from frames it never painted.

Resize and clear allocate a fresh grid and cannot alter retained frames. Cursor
visibility and position still come from the backend. Standalone and attached
windows share this path, and frame identities, acknowledgements and the
latest-frame bridge are unchanged. The new backend is private to the native
fullscreen surface; it is not a terminal emulator or scrollback implementation.

`changed_rows_are_copied_and_skipped_frames_remain_complete` in
`src/native_frontend/tests/grid.rs` verifies unchanged-row sharing, changed-row
isolation, skipped frames, unchanged redraws and resize lifetime.
`grid_backend_matches_ratatui_for_wide_cells_styles_clear_and_cursor` in the
same file compares the backend against `TestBackend` across wide/combining text,
styles, shorter replacements, cursor changes and all clear-region variants.

Known limitation: the core still prepares and renders a complete Ratatui frame.
This fix removes the publication copy; it does not remove layout or cell diff
work, which belongs to separate rendering optimisations.

## Report

Every frame the host publishes to the native window (`--window`) copies the
entire cell grid, even when one cell changed.

## Measurements

Release build, per keystroke in Insert mode on a plain-text buffer, median and
p99 over 500 keys. The host was driven directly through `WorkspaceHost`, with
no display.

| Grid | Cells | `Terminal<TestBackend>::draw` | `Buffer::clone` |
| --- | ---: | ---: | ---: |
| 120×40 | 4,800 | 0.19 / 0.37 ms | 0.07 / 0.11 ms |
| 213×54 | 11,502 | 0.37 / 0.71 ms | 0.15 / 0.21 ms |
| 426×108 | 46,008 | 0.93–1.10 / 1.65 ms | 0.40–0.50 / 0.72 ms |

Frame preparation (`prepare_frame_with_hints`) took 0.14–0.37 ms median over
the same range and is shared with the terminal frontend.

## Diagnosis

`native_frontend::Surface::draw` renders through a `ratatui::Terminal` over a
`TestBackend`. Ratatui renders into its own buffer, diffs it against the
previous one, and writes the changed cells into the `TestBackend` buffer.
`Surface::draw` then clones the complete `TestBackend` buffer into
`FrameData::cells` and hands it to the GUI thread. The diff already
identifies the few rows a keystroke changes, but the copy is always the full
grid, and the previous `FrameData` grid is freed when it is replaced.

## Expected behavior

The cost of publishing a frame follows what changed rather than the grid
size. A frame that changes a few rows copies only those rows; unchanged rows
are shared between consecutive frames. The GUI thread still receives a
complete, immutable frame that it may keep for as long as it needs.

## Constraints

- The bridge keeps only the latest frame and never queues frames; a skipped
  frame must not lose changes, so every published frame stays complete.
- Frames remain immutable once published: the GUI thread may still be
  painting a frame while the host prepares the next one.
- Painted-frame identity, attachment checks and acknowledgements are
  unchanged.
- Standalone windows and persistent-session attachments share
  `Surface::draw`, so both benefit.
- Sharing unchanged rows also lets the cell painter recognise unchanged rows
  between frames (`native_window_cell_paint_cost.md`).

## Reproduction

Time `Surface::draw` with `RUNYTE_NATIVE_PAINT_TIMING`-style instrumentation,
or reproduce the table with a release benchmark that drives
`WorkspaceHost::execute_frontend_input` and renders with
`runyte::ui::render_native_frame` into `Terminal<TestBackend>` at each grid
size.
