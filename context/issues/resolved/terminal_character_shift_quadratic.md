---
title: "Terminal character shifts repeatedly copy the same cells"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: d26624a
---

## Resolution

Commit `d26624a` (`perf(terminal): shift character ranges in one pass`).

`Grid::insert_characters` and `Grid::delete_characters` shifted an entire
remaining row once per requested character. They now use one overlapping
`copy_within` and fill only vacated cells, preserving wide-character splitting,
right-edge cleanup, cursor position, paint attributes, and wrap provenance.
Work is linear in row width rather than width multiplied by count.

`bulk_character_shifts_preserve_cells_and_paint_only_vacated_columns`,
`bulk_character_shifts_clear_split_wide_glyphs_at_both_edges`, and
`bulk_character_shifts_handle_a_maximum_width_short_screen` in
`src/terminal/grid.rs` cover normal, clamped, Unicode, attribute and wide-row
behavior. All 17 grid tests and 13 `tests/terminal_sequences.rs` tests pass.

An isolated optimized Linux comparison of four insert/delete pairs on a
32,768-column row measured 787.15 ms before and 134.94 microseconds after,
with identical resulting text. This is an adversarial-width microbenchmark,
not an ordinary-workload throughput claim.

## Report

Terminal insert-character (`CSI n @`) and delete-character (`CSI n P`)
operations repeatedly shift the same row. `Grid::insert_characters` calls
`Vec::insert` once per requested cell, and `Grid::delete_characters` calls
`Vec::remove` once per cell. Inserting or deleting a substantial fraction of a
wide row therefore copies a quadratic number of cells on the editor thread.

The requested count is bounded by the remaining row width, but the geometry
contract permits wide, short terminal panes. A single operation near column
zero can move hundreds of millions of cells at the maximum supported width.
Ordinary full-screen applications can also issue these operations repeatedly.

Both operations should shift the retained cells once, fill the vacated range
with the current background, and preserve existing wide-character cleanup and
wrap provenance. Work should grow linearly with row width rather than the
product of row width and character count.

Reproduce at the emulator boundary by populating a wide row, positioning the
cursor near its beginning, and issuing `CSI 16000 @` or `CSI 16000 P` in a
32768-column, one-row terminal. Smaller deterministic cases should verify the
exact shifted content, background cells, count clamping, and wide characters
crossing the insertion, deletion, or right-edge boundary.
