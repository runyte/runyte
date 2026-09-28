# P2 — Cursor movement repeats unchanged syntax highlighting

Priority: P2 (normal). This affects ordinary movement in source files, but the
measured cost alone does not establish perceptible input lag.

At commit `22fd664`, `App::snapshot_pane` in `src/snapshot.rs` constructs
highlight ranges on every frame, including frames where only the cursor moved.
Ordinary adjacent lines generally remain separate ranges because their newline
characters lie between the queried ranges. Each call to `App::highlights`
reaches `DocumentSyntax::spans` in `src/syntax/mod.rs`, constructing a new
highlighter. There is no reuse of the previous frame's visible spans here.

An exploratory measurement on 2026-09-28 used Linux x86-64, an AMD Ryzen AI 9 365,
and a library built with `cargo build --release --locked --lib`. The fixture
contained 2,000 Lua lines of the form
`local function item_{i}(x) return x + {i} end\n`, with zero-based `i`.
After ten warmups, 200 samples gave a median of 247.2 microseconds for 38
separate line queries and 189.9 microseconds for one contiguous query covering
the same rows, including their newlines. Repeated `HeadlessEditor::snapshot`
calls took 339.2 microseconds at 120×40 and 708.9 microseconds at 240×80.
The snapshot figures include other frame preparation work; they are not an
isolated measure of highlighting. These are internal timings, not terminal
input latency or a comparison with Helix.

To reproduce, construct and parse that fixture with `Registry`, `Text`, and
`DocumentSyntax`. Compare `spans` calls from each row's start through its
`line_len` for rows 0 through 37 against one call from offset zero to
`line_to_offset(38)`. Separately open the fixture through `HeadlessEditor` and
measure repeated snapshots after warming the viewport. Use temporary storage.

Cursor-only frames should reuse unchanged visible highlight spans. A candidate
fix is caching by buffer identity, text and syntax revisions, and visible
ranges, with safe merging of adjacent visible-line queries. Invalidation must
cover asynchronous syntax publication and stale-syntax translation as well as
edits. Preserve bounded work for long lines and disjoint ranges across folds;
merging must not highlight a large hidden region.

Validation should compare scopes before and after edits, scrolling, folding,
and background syntax publication, and measure repeated cursor-only frames.
Unchanged-pane reconstruction is tracked separately in
`performace_unchanged_pane_snapshots.md`.
