---
title: "Soft-wrap cache capacity was smaller than ordinary viewports"
status: resolved
reported: 2026-09-28
resolved: 2026-09-28
commit: e8a52ec
---

## Resolution

Commit `e8a52ec` (`Keep visible soft-wrap layouts resident in ordinary viewports`)
raised the per-buffer cache limit in `wrap::line_segments` from 16 to 256
layouts. The 16-entry LRU limit evicted short logical lines while projecting a
viewport taller than 16 lines, so the next unchanged frame recomputed their
segments. Two views of one buffer at different widths put separate layouts in
the same cache and made that eviction more likely.

The larger entry limit retains ordinary visible working sets while the existing
8 MiB segment-payload ceiling still evicts layouts as needed. Cache keys still
include the text revision, logical row, pane width, and tab width; edits, undo,
redo, and geometry changes cannot reuse stale layouts. Oversized layouts are
computed without retention. In a 100-line, two-width fixture, the original
16-entry limit recomputed 600 of 600 repeated lookups over three steady frames;
the 256-entry limit recomputed none. This count measures cache reuse, not total
snapshot or cursor-movement time.

Coverage in `src/wrap/tests/cache.rs` is
`ordinary_viewports_keep_geometry_across_frames_scrolls_and_pane_widths` for
steady frames, scroll overlap, and two widths;
`cache_eviction_and_oversized_geometry_preserve_results` for entry and segment
bounds and uncached long lines;
`repeated_edits_and_tab_width_changes_do_not_retain_stale_geometry` for repeated
revision changes and tab-width keys;
`geometry_is_reused_across_panes_and_invalidated_by_edit_undo_and_redo` for
shared panes, width changes, edits, undo, and redo; and
`cached_geometry_preserves_unicode_tabs_and_word_boundaries` for segment
correctness across Unicode, tabs, widths, and word boundaries.

Known limitation: a working set larger than 256 layouts can still thrash while
retained segment payload remains below 8 MiB. The measured cache misses show
reuse in the tested fixture, not a demonstrated improvement in frame latency.

## Report

Priority: P3 (low). The eviction pattern follows from the implementation;
significant latency attributable to it has not been demonstrated.

At commit `22fd664`, `line_segments` in `src/wrap.rs` retained at most 16 logical
lines in a per-buffer cache, in addition to its segment memory bound. A typical
viewport can display more than 16 distinct logical lines. Sequentially
projecting those lines could evict earlier entries before the next frame
revisited them, causing geometry to be recomputed despite unchanged text.
Multiple views of a buffer at different widths could increase cache pressure.

This finding applied when soft wrapping was enabled; the default configuration
had it disabled. A long logical line occupying many screen rows was a different
case and could benefit from the existing cache. Exploratory 120×40 snapshot
medians on 2026-09-28 were 96.3 microseconds for 80-character lines and 80.5
microseconds for 10,000-character lines, with wrapping enabled. Those fixtures
showed different counts of logical lines and did not isolate eviction cost or
prove a noticeable slowdown.

To reproduce the eviction pattern, enable wrapping on at least 100 short lines
and use a viewport displaying more than 16 distinct logical lines. Warm the
view, then count segment recomputations during repeated frames or `h`/`l`
movement without scrolling. Compare with a viewport containing at most 16
logical lines and with two panes showing the same buffer at different widths.
Measure cache misses separately from total frame time in a release build.

The expected cache behavior was to retain the working set of visible logical
lines where the memory bound permitted. Capacity or eviction policy could
account for visible panes while preserving a strict memory ceiling and
revision/width/tab-width correctness. Edits, undo, and geometry changes must
not reuse stale segments. Validation needed to cover steady cursor-only frames,
scrolling, differently sized panes, long lines, and repeated edits without
unbounded retained geometry.
