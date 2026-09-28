# P3 — Soft-wrap cache capacity is smaller than ordinary viewports

Priority: P3 (low). The eviction pattern follows from the implementation;
significant latency attributable to it has not been demonstrated.

At commit `22fd664`, `line_segments` in `src/wrap.rs` retains at most 16 logical
lines in a per-buffer cache, in addition to its segment memory bound. A typical
viewport can display more than 16 distinct logical lines. Sequentially
projecting those lines can evict the earlier entries before the next frame
revisits them, causing geometry to be recomputed despite unchanged text.
Multiple views of a buffer at different widths can increase cache pressure.

This finding applies when soft wrapping is enabled; the default configuration
has it disabled. A long logical line occupying many screen rows is a different
case and can benefit from the existing cache. Exploratory 120×40 snapshot
medians on 2026-09-28 were 96.3 microseconds for 80-character lines and 80.5
microseconds for 10,000-character lines, with wrapping enabled. Those fixtures
show different counts of logical lines and do not isolate eviction cost or
prove a noticeable slowdown.

To reproduce the eviction pattern, enable wrapping on at least 100 short lines
and use a viewport displaying more than 16 distinct logical lines. Warm the
view, then count segment recomputations during repeated frames or `h`/`l`
movement without scrolling. Compare with a viewport containing at most 16
logical lines and with two panes showing the same buffer at different widths.
Measure cache misses separately from total frame time in a release build.

The cache should retain the working set of visible logical lines where the
memory bound permits. Capacity or eviction policy can account for visible
panes, while preserving a strict memory ceiling and revision/width/tab-width
correctness. Edits, undo, and geometry changes must not reuse stale segments.

Validation should cover steady cursor-only frames, scrolling, differently sized
panes, long lines, and repeated edits without unbounded retained geometry.
