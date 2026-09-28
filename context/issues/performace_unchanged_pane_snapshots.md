# P2 — Cursor movement rebuilds unchanged visible panes

Priority: P2 (normal). The work affects every ordinary redraw and scales with
visible panes; its separate contribution to input latency is not yet measured.

Code inspection at commit `22fd664` found that `App::prepare_view` in
`src/app/presentation.rs` prepares the visible panes and `App::snapshot` in
`src/snapshot.rs` reconstructs each pane's owned presentation rows on every
frame. Moving a cursor in one pane therefore also reconstructs content for
other visible panes whose text, viewport, selection, and decorations have not
changed. `WorkspaceHost::prepare_frame_with_hints` in `src/workspace/host.rs`
uses this path for both standalone and persistent modes.

The expected behavior is to reuse unchanged pane content while updating the
active caret, selection, status, and any independently changed decorations.
This is a code-confirmed work pattern, not a demonstrated multi-pane latency
regression. Single-pane exploratory timings from 2026-09-28 do not isolate
the cost of rebuilding inactive panes.

To reproduce, open an ordinary source file in one pane, then add visible panes
showing other unchanged source buffers. After syntax settles, repeat `h` and
`l` in the active pane without scrolling. Measure preparation and snapshot
cost separately from drawing for one, two, and four panes at fixed total
terminal dimensions, and count snapshot reconstruction for inactive panes.
Repeat with multiple panes showing the same buffer.

A candidate fix is revision-based reuse of prepared rows or immutable pane
content. Invalidation must account for geometry, wrapping, folds, themes,
diagnostics, Git marks, search previews, mode, focus, and shared-buffer edits.
Cursor motion must still update cursor-line presentation and status correctly.
Background events must remain able to refresh inactive panes.

Validation should establish reuse for unchanged panes and correct invalidation
for those presentation changes. Highlight-span reuse is a related but distinct
task in `performace_cursor_highlighting.md`; transport of complete frames is
tracked in `performace_persistent_cursor_frames.md`.
