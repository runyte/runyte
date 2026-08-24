---
title: "Fast mouse scrolling can freeze an attached integrated terminal"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: f9d168d
---

## Resolution

Commit f9d168d (`Keep terminal wheel bursts responsive`) fixed a transport
feedback loop exposed by children such as Claude that enable SGR mouse
reporting. The attached-client loop in `run_attached` was synchronously sending
one protocol request for every physical wheel report. `run_host_server` then
treated every forwarded report, including a stale one, as a visual change and
published another editor frame. That advanced the exact frame identity required
by `WorkspaceHost::execute`, made queued reports stale, and created enough
opposing socket and frame pressure to wedge the attachment. Every accepted
report also occupied its own entry in the terminal's small PTY input queue.

The attached client now coalesces consecutive identical wheel reports at an
eight-millisecond cadence and carries a bounded repetition count through
protocol version 24. `App::handle_pointer_repeated` preserves the full scroll
distance and `TerminalSession::send_mouse_repeated` writes the corresponding
SGR reports as one bounded PTY chunk. Forwarding a report to a child is now an
explicit non-visual host outcome, so it does not publish an editor frame.
Prepared frames also retain pointer compatibility while their hit-test view is
unchanged, allowing terminal-output-only frame advances without invalidating
otherwise current pointer input. A change of layout or viewport still resets
that compatibility boundary, and clicks, drags, direction changes, keyboard
input, and resize events flush a pending wheel batch to preserve ordering.

The original disposable persistent-PTY reproduction froze after about 3,324
wheel reports in two seconds. The same shape remained responsive after 3,599
reports with the fix, and a higher-rate pass remained responsive after 12,944
reports.

Tests covering the behavior are:

- `app::tests::pointer_click_drag_wheel_and_resize_use_the_prepared_projection`
  in `src/app.rs`.
- `app::tests::a_reported_terminal_click_focuses_its_pane_before_forwarding`
  in `src/app.rs`.
- `tests::attached_pointer_batcher_coalesces_only_identical_wheel_input` in
  `src/main.rs`.
- `protocol::tests::protocol_version_and_request_bounds_are_explicit` in
  `src/protocol/mod.rs`.
- `terminal::tests::sgr_mouse_encoding_preserves_coordinates_buttons_and_modifiers`
  in `src/terminal/mod.rs`.
- `workspace::host::tests::pointer_frames_remain_compatible_until_the_prepared_view_changes`
  in `src/workspace/host.rs`.

## Report

When Claude runs in an integrated terminal, scrolling the mouse to move up and
down sometimes freezes Runyte. Recovering requires restarting Runyte and
reconnecting to the host. Scrolling heavily and quickly makes the freeze more
likely.
