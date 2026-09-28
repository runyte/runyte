---
title: "Cursor movement repeats unchanged syntax highlighting"
status: resolved
reported: 2026-09-28
resolved: 2026-09-28
commit: b9bd9a3
---

## Resolution

Commit `b9bd9a3` (`Reuse visible syntax spans on cursor frames`) resolves the
repeated syntax query cost. `App::snapshot_pane` in `src/snapshot.rs` rebuilt
visible highlight ranges on every frame and passed most lines separately to
`App::highlights`. Each call to `DocumentSyntax::spans` constructed a new
highlighter even when only the cursor or selection presentation had changed.

The pane now retains its last visible syntax spans under the buffer identity,
text revision, syntax document identity and revision, current versus stale tree
state, and exact merged visible ranges. A replacement tree can have the same
syntax revision as its predecessor, so its document identity is part of the
key. Edits change the text revision; a retained stale tree and its published
replacement have distinct source states, so background publication cannot
reuse translated spans. Generated-page and Git branch semantic highlights
continue through their existing paths without this syntax cache.

Adjacent visible rows are queried together only when their ranges are
separated by one newline. Gaps across folds and clipped long lines remain
separate, so the highlighter does not traverse a hidden fold or the unseen
remainder of a long line. The cache is bounded to one visible result per pane
and discards entries for panes absent from the prepared view.

An optimized 2,000-line Lua fixture used ten warmups and 200 samples per case.
At 120×40, median snapshots took 390 microseconds with the cache cleared each
frame and 128 microseconds with it warm. At 240×80, the corresponding medians
were 830 and 278 microseconds. These are same-build comparisons of snapshot
construction, not terminal input latency; the original exploratory timings
below used a different commit and measurement path.

`src/snapshot.rs` covers reuse, exact scope agreement between merged and
separate visible-line queries, edits, tree replacement, and scrolling in
`cursor_highlights_reuse_visible_spans_and_invalidate_on_edits_trees_and_scroll`.
Its `visible_queries_skip_folded_lines_and_clip_a_long_line` covers fold gaps,
unfolding, and a 100,000-character line. The ignored manual timing fixture is
`cursor_highlight_snapshot_timing` in the same file.
`tests/background_syntax.rs` covers the transition from translated stale spans
to published comment scopes in
`stale_tree_exposes_translated_spans_but_no_structure_until_drain`.
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and the full
native `cargo test` suite passed on Linux.

Known limitation: cache reuse requires the complete merged visible range set
to match. Scrolling recomputes spans for the new viewport even where rows
overlap. Row reconstruction within an unchanged pane is tracked separately in
`performace_unchanged_pane_snapshots.md`.

## Report

Priority: P2 (normal). This affects ordinary movement in source files, but
the measured cost alone did not establish perceptible input lag.

At commit `22fd664`, `App::snapshot_pane` in `src/snapshot.rs` constructed
highlight ranges on every frame, including frames where only the cursor moved.
Ordinary adjacent lines generally remained separate ranges because their
newline characters lay between the queried ranges. Each call to
`App::highlights` reached `DocumentSyntax::spans` in `src/syntax/mod.rs` and
constructed a new highlighter. The previous frame's visible spans were not
reused.

An exploratory measurement on 2026-09-28 used Linux x86-64, an AMD Ryzen AI 9
365, and a library built with `cargo build --release --locked --lib`. The
fixture contained 2,000 Lua lines of the form
`local function item_{i}(x) return x + {i} end\n`, with zero-based `i`.
After ten warmups, 200 samples gave a median of 247.2 microseconds for 38
separate line queries and 189.9 microseconds for one contiguous query covering
the same rows, including their newlines. Repeated `HeadlessEditor::snapshot`
calls took 339.2 microseconds at 120×40 and 708.9 microseconds at 240×80.
The snapshot figures included other frame preparation work; they were not an
isolated measure of highlighting. These were internal timings, not terminal
input latency or a comparison with Helix.

To reproduce, construct and parse that fixture with `Registry`, `Text`, and
`DocumentSyntax`. Compare `spans` calls from each row's start through its
`line_len` for rows 0 through 37 against one call from offset zero to
`line_to_offset(38)`. Separately open the fixture through `HeadlessEditor` and
measure repeated snapshots after warming the viewport. Use temporary storage.

The expected behavior was to reuse unchanged visible spans on cursor-only
frames. The report proposed keys for buffer identity, text and syntax
revisions, and visible ranges, plus safe merging of adjacent visible-line
queries. Invalidation needed to cover asynchronous syntax publication,
stale-syntax translation, and edits. Long lines needed bounded queries;
disjoint ranges across folds could not draw a large hidden region into a
highlight query. Validation called for comparing scopes before and after
edits, scrolling, folding, and background syntax publication, then timing
repeated cursor-only frames. Unchanged-pane reconstruction was tracked
separately in `performace_unchanged_pane_snapshots.md`.
