---
title: "Long errors were clipped in the interaction line"
status: resolved
reported: 2026-08-16
resolved: 2026-08-16
legacy_commit: c238d3b
---

## Resolution

Commit `c238d3b` (`Add workspace notification center`) resolved this. The
interaction line was previously fed by `App::status` and `App::error` through
`StatusSnapshot::message`; `ui::draw_status` rendered that value into one
terminal row, so Ratatui correctly clipped everything past the pane width and
there was no durable place from which to recover the missing text.

`notification::NotificationCenter` now owns bounded, workspace-lifetime
ERROR, WARNING, and INFO entries independently of action echo. It retains
bounded multiline bodies newest first, coalesces consecutive identical entries, tracks
unacknowledged event sequence, and renders the single searchable read-only
`[notifications]` buffer opened by `:notifications` or `:not`. Opening that
buffer acknowledges the currently retained entries; later inserts refresh it
without changing the active pane. The default 50-entry limit is the typed
`notifications.history_limit` setting with a validated range of 1 through
1000.

The retained representation also has independent byte bounds: 1 MiB per entry
and 8 MiB across a workspace. Truncation is written into the retained text,
and `App::refresh_notification_buffers` avoids constructing the document when
no notification buffer is open. These bounds prevent the configurable count
limit from multiplying large third-party logs into unbounded editor-thread
allocations.

`App::error`, unavailable command paths, host errors, asynchronous Git
failures, and LSP status/error boundaries now create notifications with
Runyte-owned severity and source. Routine LSP readiness remains silent rather
than creating an INFO entry merely because a successful lifecycle transition
occurred. The interaction line now carries only an active prompt or action
echo, and failed bindings point to `:not` without background notifications
replacing the echo.

Asynchronous Git requests retain the interaction identity that initiated them.
A delayed failure amends that echo only while it is still current, and a
successful result updates the same correlated echo. Multiline success output,
or useful output that arrives after its echo was superseded, becomes an INFO
notification rather than disappearing.

The immutable snapshot and workspace protocol now carry unread severity counts
and notification-row severity. The global status line renders nonzero `E`,
`W`, and `I` counts in semantic theme colors, compacting to highest severity
plus total when width is constrained. The indicator is right-anchored so
narrow status content cannot clip it, and it remains visible while a
long-running action owns the rest of the row. The protocol advanced from version 10
to 11 because the frame theme, status, row, interaction-line, and input
geometry shapes changed. The adopted names for the Runyte screen, editor area,
panes, gutter, content padding, buffer viewport, global status line,
interaction line, and overlays are recorded in
`context/reference/ui-vocabulary.md`.

Git failures retain both labelled stdout and stderr inside an explicit 1 MiB
combined budget, so hook output does not disappear merely because it used the
other stream. Each truncated stream carries a visible marker.

Coverage is provided by
`notification::tests::history_is_newest_first_bounded_and_acknowledged`,
`notification::tests::consecutive_duplicates_coalesce_and_become_unread_again`,
and `notification::tests::document_preserves_multiline_details_and_escapes_controls`
in `src/notification.rs`;
`app::tests::notification_commands_open_one_complete_buffer_and_acknowledge_history`
and
`app::tests::counted_colon_binding_echoes_failure_and_retains_its_info_notification`
in `src/app.rs`;
`snapshot::tests::a_later_host_failure_does_not_replace_completed_binding_feedback`
in `src/snapshot.rs`;
`config::tests::notification_history_defaults_to_fifty_and_is_bounded` and
`config::tests::older_themes_derive_notification_colors_from_change_roles` in
`src/config.rs`; and
`git::cli::tests::long_failure_logs_are_large_and_explicitly_bounded` in
`src/git/cli.rs`. Follow-up coverage includes delayed Git action correlation,
multiline Git success output, notification byte bounds without an open buffer,
narrow and long-running status rendering, protocol-v11 notification values,
and persistent-host detach/reattach state. Run them together with `cargo test`.

Known limitation: safety bounds necessarily truncate pathological producer
output. The retained text always states when that happened.

## Report

Long errors and warnings occupied the interaction line below the global status
line and were clipped to the terminal width. The complete message needed to
remain readable, including multiline third-party output.

The interaction line was to be reserved for active prompts and the last action
echo; notifications were not to replace either. Failed actions were to leave a
compact echo pointing to `:not`.

Workspace-lifetime notifications needed ERROR, WARNING, and INFO severities
assigned by Runyte for each producer and use case. Notification creation was a
separate decision from severity assignment: routine successful polling and
silent successful commands were not to flood the history. Notifications were
to queue without stealing focus.

The newest 50 notifications were to be retained by default, configurable from
1 through 1000. They were to appear newest first in one non-paged, searchable,
read-only `[notifications]` buffer opened by `:notifications` or `:not`, with
complete multiline details, local timestamps, source and title. Consecutive
identical entries were to coalesce with an occurrence count. Opening the buffer
was to acknowledge everything currently retained; later entries were unread.

The global status line was to show nonzero unread `E`, `W`, and `I` counts in
semantic theme colors and compact narrow displays to highest severity plus
total. The notification history belonged to editor state so a persistent
workspace host retained it across TUI detach and reattach, but it was not to be
persisted across host restart.
