---
title: "Git projection buffers did not refresh automatically"
status: resolved
reported: 2026-08-11
resolved: 2026-08-12
legacy_commit: 0a122fa
---

## Resolution

Commit `0a122fa` (`Move editor Git work off the frame`) resolved this report.

Git commands and projection refreshes previously called `GitProvider`
synchronously from `App`; there was no host-owned timer, and adding one there
would have run Git on the input/render thread. The change introduced
`git::service::GitService`, which orders mutations per common repository,
coalesces equivalent refreshes, executes subprocesses away from the host
thread, and returns owned snapshots for host-side application. The
`WorkspaceHost::refresh_git_if_due` timer requests those snapshots only while a
Git-derived view or tracked-file gutter is visible.

The new typed `git.refresh_interval_seconds` setting defaults to five seconds,
is available through the settings registry and `config.example.yaml`, and
accepts zero to disable periodic refresh. Status selection is retained by path,
branch selection by ref name, and index/diff selection by unified-diff hunk and
line identity. Refresh failures retain the last known projection and mark it
stale instead of clearing it.

Coverage includes `git::service::tests::equivalent_refreshes_coalesce_onto_one_worker`
in `src/git/service.rs`,
`app::tests::the_caret_follows_the_file_it_was_on_across_a_refresh` and
`app::tests::a_refreshed_diff_follows_the_same_line_in_the_same_hunk` in
`src/app.rs`, and the settings registry validation tests in `src/settings.rs`.

## Report

The Git status buffer opened with `Space g g` did not refresh automatically.
It was expected to refresh every five seconds by default, with the interval
configurable in the Runyte configuration. The Git branches and Git index
buffers were expected to follow the same refresh behavior.
