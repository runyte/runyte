---
title: "Terminal line operations perform quadratic row shifts"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 2e742e2
---

## Resolution

Commit `2e742e2` (`perf(terminal): shift line regions in one pass`) replaces
repeated vector removal and insertion in `Grid` with one rotation of the
affected row and identity slices, followed by replacement of the vacated rows.
The upward helper retires rows in their original order only for top-anchored
primary scrolling. The downward helper allocates blank row identities in reverse
position order to preserve the identity ordering of repeated single-line calls.
Delete-line continues to discard its removed rows without adding history.

`benchmarks/terminal_grid.py` compares actual before/after grid modules with
optimized Rust builds and verifies identical final text outside the timed
interval. Three measured samples on this Linux machine gave a median of
1216.661 ms before and 0.602435 ms after for four operations over a one-column,
32768-row grid, each moving 16000 rows. This is an extreme supported geometry,
not a measurement of ordinary PTY throughput or rendering latency. The harness
records compiler, source hashes, platform and individual samples for reruns.

Regression tests in `src/terminal/grid.rs` are
`bulk_line_operations_preserve_single_line_semantics_and_provenance`,
`bulk_scroll_retains_only_newest_history_and_delete_lines_retains_none`, and
`bulk_line_operations_handle_a_maximum_height_narrow_screen`. All 20 grid tests
and all 13 `tests/terminal_sequences.rs` tests passed. An independent reviewer
checked the region bounds, row identity ordering, history policy and tests.

## Report

Terminal scroll-up, scroll-down, insert-line and delete-line operations shift
the row vectors once per requested line. `Grid::scroll_up`, `scroll_down`,
`insert_lines` and `delete_lines` use repeated `Vec::remove`/`Vec::insert` on
both cells and row identities. On a tall, narrow supported geometry, one
large line count therefore performs quadratic work on the editor thread.

Scrolling or editing a region should move its retained rows once and replace
the vacated rows in linear time. Lines outside the region, cursor behavior,
stable row identities, wrap provenance, background colours, and the bounded
scrollback order must remain intact. Only scroll-up in a top-anchored primary
region should retire rows to history; delete-line must continue discarding
its removed rows.

Reproduce through a one-column, 32768-row emulator by issuing a large `CSI n S`,
`CSI n T`, `CSI n L`, or `CSI n M`. Counts remain clamped to the affected region.
Boundary coverage should include partial regions, zero and excessive counts,
both history policies, repeated scrollback eviction, and wrapped row identity.
