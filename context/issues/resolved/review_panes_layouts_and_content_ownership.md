---
title: "Pane bulk operations lacked direct buffer-retention coverage"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: c1d0def
---

## Resolution

Commit c1d0def (`Cover pane and buffer ownership boundary`) completed the pane,
layout, and content-ownership audit without finding a production defect. It
added direct state-transition coverage for the highest-risk ownership
boundary: `only_window` removes inactive pane views and collapses the layout,
but does not close or retire the buffers those views uniquely displayed.

The regression uses three distinct path-backed buffers in a nested three-pane
layout. After the bulk view operation it verifies both inactive pane IDs are
gone, all buffer IDs remain live and path-associated, and a buffer whose only
pane was removed can immediately be switched into the surviving pane.

Coverage lives in `src/app/tests/commands.rs` in
`only_window_drops_views_without_retiring_their_buffers`.

## Report

The pane tree and ownership and lifecycle of content shown in panes required a
focused hardening review. The scope included `src/layout.rs`, pane and buffer
state in `src/app.rs`, `src/app/presentation.rs`, file and terminal workflows,
`src/diff_view.rs`, and their tests.

The review covered recursive layout invariants, minimum geometry,
active-pane validity, recency-aware focus, splits and closes, shared buffers,
last-pane rules, terminal ownership, special-buffer retention, comparison
pairs, maximized and Zen modes, resize operations, workspace switching, stale
content identifiers, and cleanup after replacement or failure. No confirmed
production defect was found; the missing direct bulk-view ownership regression
was retained to make that invariant explicit.
