---
title: "Cursor movement rebuilds unchanged visible panes"
status: resolved
reported: 2026-09-28
resolved: 2026-09-28
commit: 37fa5bb
---

## Resolution

Commit `37fa5bb` (`Reuse unchanged inactive pane snapshots`) resolves the
repeated reconstruction of inactive document panes. `App::snapshot` in
`src/snapshot.rs` previously called `snapshot_pane` for every prepared pane on
every frame. Moving the active caret therefore rebuilt owned rows for other
panes even when their content and presentation had not changed.

`App::snapshot_cached_pane` retains one owned snapshot per visible inactive
document pane and reuses it when the prepared projection and its presentation
inputs still match. The key covers buffer and syntax revisions, generated
highlights, geometry and projected rows, title and caret position, mode and
focus, theme and relevant editor settings, row hints, diagnostics, Git marks,
and comparison decorations. Text edits to a buffer shared by several panes
change its revision and invalidate each view. The cache drops panes absent
from the prepared view; returned snapshots clone their rows and remain owned
by the caller. Active panes retain the live path for caret, selection, search
preview, cursor-line, and inline diagnostic presentation. Terminal panes and
typed list buffers also retain their live paths because their changing
presentation is not fully described by the document cache key.

An optimized, fixed 120×40 fixture measured 200 alternating `h` and `l`
motions in the active pane, with Lua syntax ready. The table gives median
microseconds for **forced cache misses** versus **warmed cache hits in the
new implementation**. The forced-miss case clears the pane cache each frame
but still pays for key construction, insertion, and cloning; it is not a
measurement of the parent commit. Drawing and input dispatch were outside
both timed sections. Rebuild counts include the initial warmup frame.

| Visible panes | `prepare_view` miss / warm | `snapshot` miss / warm | Inactive rebuilds miss / warm |
| --- | ---: | ---: | ---: |
| 1 | 0 / 0 µs | 83 / 84 µs | 0 / 0 |
| 2, distinct buffers | 1 / 1 µs | 171 / 101 µs | 201 / 1 |
| 2, shared buffer | 1 / 1 µs | 174 / 103 µs | 201 / 1 |
| 4, distinct buffers | 2 / 2 µs | 202 / 59 µs | 603 / 3 |
| 4, shared buffer | 2 / 2 µs | 206 / 60 µs | 603 / 3 |

`src/snapshot.rs` tests
`inactive_panes_reuse_owned_rows_while_active_cursor_and_status_move` for
two and four panes, current status and caret, owned rows, and cache pruning;
`inactive_pane_cache_invalidates_for_shared_text_projection_theme_and_diagnostics`
for shared-buffer edits, geometry, wrapping, theme, whitespace, syntax
publication, mode, diagnostic publication, and focus; and the ignored
`inactive_pane_snapshot_timing` for the measurements above. The
`inactive_pane_snapshot_refreshes_after_git_marks_change_without_text_edit`
test in `src/app/tests/presentation_and_settings.rs` covers background Git
mark changes. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
and the full native `cargo test` suite passed.

Known limitation: conservative global Git and diagnostic revisions can
rebuild unrelated inactive document panes when either feed changes.

## Report

Priority: P2 (normal). The work affected every ordinary redraw and scaled
with visible panes; its separate contribution to input latency had not been
measured when reported.

Code inspection at commit `22fd664` found that `App::prepare_view` in
`src/app/presentation.rs` prepared the visible panes and `App::snapshot` in
`src/snapshot.rs` reconstructed each pane's owned presentation rows on every
frame. Moving a cursor in one pane therefore also reconstructed content for
other visible panes whose text, viewport, selection, and decorations had not
changed. `WorkspaceHost::prepare_frame_with_hints` in `src/workspace/host.rs`
used this path for both standalone and persistent modes.

The expected behavior was to reuse unchanged pane content while updating the
active caret, selection, status, and any independently changed decorations.
This was a code-confirmed work pattern, not a demonstrated multi-pane latency
regression. Single-pane exploratory timings from 2026-09-28 did not isolate
the cost of rebuilding inactive panes.

To reproduce, open an ordinary source file in one pane, then add visible panes
showing other unchanged source buffers. After syntax settles, repeat `h` and
`l` in the active pane without scrolling. Measure preparation and snapshot
cost separately from drawing for one, two, and four panes at fixed total
terminal dimensions, and count snapshot reconstruction for inactive panes.
Repeat with multiple panes showing the same buffer.

The proposed approach was revision-based reuse of prepared rows or immutable
pane content. Invalidation needed to account for geometry, wrapping, folds,
themes, diagnostics, Git marks, search previews, mode, focus, and
shared-buffer edits. Cursor motion still had to update cursor-line
presentation and status correctly. Background events still had to refresh
inactive panes.

Validation called for establishing reuse for unchanged panes and correct
invalidation for those presentation changes. Highlight-span reuse was a
related but distinct task in `performace_cursor_highlighting.md`; transport
of complete frames was tracked in `performace_persistent_cursor_frames.md`.
