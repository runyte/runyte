---
title: "Diff viewport mapping scans entire filler gaps"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 4cca101
---

## Resolution

Commit `4cca101` (`perf(diff): locate filler rows by alignment run`).

`DiffSession::row_at_or_above` walked backward through every filler row,
performing a run lookup each time. It now finds the containing alignment run
and uses its contiguous side range to obtain the preceding row directly.
Leading gaps, empty sides and virtual rows retain their existing mapping.

Coverage: `filler_lookup_matches_backward_scan_for_every_small_alignment` and
`large_inserted_block_maps_directly_to_preceding_row` in `src/diff_view.rs`;
all nine module tests passed. `benchmarks/diff_filler.rs` compares the old scan
and current modules outside alignment construction. Five measured samples of
100 lookups across a million-row gap had medians of 146.793 ms and 0.000450 ms.
This is isolated algorithmic stress, not an end-to-end scrolling measurement.

## Report

`DiffSession::row_at_or_above` searches backward through every aligned row until
it reaches a real row of the requested side. Each candidate performs a binary
search through the alignment runs. A large inserted or deleted block therefore
requires work proportional to the entire filler gap on every viewport mapping,
even though the run already records the preceding document row.

For example, compare `head\ntail\n` against `head\n` followed by one million
`x\n` lines and `tail\n`. Both inputs fit the 4 MiB comparison limit. Mapping
the left side near the end of the inserted block scans one million filler
positions to return row zero. Linked diff viewport preparation invokes this
mapping on the editor thread, so moving through large one-sided changes can
delay rendering.

Find the containing run directly and use its side range to return the nearest
real row. Preserve actual rows, leading filler with no preceding row, unequal
replacement runs, empty sides, and the existing virtual rows past the alignment.
The result must match the backward scan without visiting each filler position.
