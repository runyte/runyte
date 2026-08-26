---
title: "Snapshot geometry treated combining marks as visible cells"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: 1a49f97
---

## Resolution

Commit 1a49f97 (`Measure snapshot geometry in display cells`) removed the
assumption that every character occupies at least one terminal cell. Wrapping,
snapshot rows, prompt cursors, row hints, and centered content now consistently
measure combining marks as zero-width while retaining marks attached to the
last visible base character. Wide glyphs and tabs remain indivisible at the
viewport boundary.

The snapshot renderer and highlight collector also bound the number of
zero-width characters scanned beyond a viewport. The bound applies to both
unwrapped and soft-wrapped rows, preventing a hostile all-combining-mark line
from turning frame preparation into work proportional to the full line.
Padding-only row hints are now omitted instead of producing a phantom hint.

Coverage lives in `src/content_alignment.rs` in
`combining_marks_do_not_widen_centered_content`, `src/row_hints.rs` in
`combining_marks_are_zero_width_and_padding_without_a_hint_is_omitted`,
`src/wrap.rs` in `combining_marks_do_not_consume_cells_or_force_wrap`, and
`src/snapshot.rs` in `combining_marks_do_not_clip_the_last_visible_cell`,
`zero_width_only_rows_have_a_bounded_scan`, and
`soft_wrapped_zero_width_only_rows_have_a_bounded_snapshot`.

## Report

Presentation-neutral snapshots, Ratatui rendering, and row and cell geometry
required a focused hardening review while preserving the boundary between
core state and frontends. The scope included `src/snapshot.rs`, `src/ui.rs`,
`src/wrap.rs`, `src/content_alignment.rs`, `src/row_hints.rs`, related
presentation code, and snapshot or geometry tests.

The review covered tiny and zero-sized regions, clipping, scrolling, viewport
bounds, soft wrapping, tabs, wide and combining characters, folds, selections,
carets, overlays, lists and prompts, comparison alignment, terminal cells,
stale snapshot data, hidden versus active panes, and frame-time or allocation
growth. Regression coverage was required at semantic snapshot and focused
geometry boundaries rather than broad full-frame snapshots.
