---
title: "Newly opened file text shifts right after the first render"
status: resolved
reported: 2026-08-12
resolved: 2026-08-12
legacy_commit: 92853a0
---

## Resolution

Commit 92853a0 (`Keep file text aligned while Git gutter loads`) corrected the
gutter geometry used while a newly opened file waits for its asynchronous Git
staged-content result. `App::prepare_view` was adding the Git change-mark
column only after `GitTracker::tracks` became true, so that delayed result
reduced the text viewport and moved every rendered line one cell to the right.

The line-number separator now always owns a one-cell margin before the text.
When Git change tracking is available, its marker occupies that existing cell
instead of adding another one. `ui::snapshot_line` renders a blank cell there
when no Git marker is present, keeping the geometry, wrapping, pointer mapping,
and visible first frame aligned with the later tracked frame.

Coverage is provided by
`delayed_git_tracking_reuses_the_line_number_text_margin` in `src/app.rs` and
`first_frame_keeps_a_text_margin_after_the_line_number_separator` in
`src/ui.rs`.

## Report

When a file was opened for the first time, its text initially touched the
vertical separator between the line numbers and the text. About a second later,
the text moved one character to the right. The expected behavior was for the
first render to include that one-character space immediately.
