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

The typed `git.refresh_interval_seconds` setting is available through the
settings registry and `config.example.yaml`. A later refinement made filesystem
invalidation the primary trigger and changed the default to 60 seconds as a
maximum-staleness fallback; zero now disables both watcher-triggered and
fallback refresh. Status selection is retained by path, branch selection by ref
name, and index/diff selection by unified-diff hunk and line identity. Refresh
failures retain the last known projection and mark it stale instead of clearing
it.

A later completion-aware refinement made narrow snapshots merge only their
requested staged bases and statistics. Automatic freshness is now recorded on
successful snapshot completion rather than submission, so a failed or
coalesced read cannot suppress the reconciliation that still needs to run.
The freshness timestamp is taken before the first Git read, and staged bases
are retired when their last file buffer closes; a late asynchronous response
cannot restore a base with no live consumer.
Partial status and staged-content reads no longer clear the repository-wide
stale indication, and save-as retires the previous path's base before tracking
the new path.
Confirmed explorer filesystem plans now request Git reconciliation directly,
and moves retire and re-track affected open-file bases even when automatic
monitoring is disabled.
Their asynchronous reconciliation is a non-coalescing post-change barrier;
one conflated snapshot is retained and retried if the bounded service queue is
temporarily full.

Coverage includes `git::service::tests::equivalent_refreshes_coalesce_onto_one_worker`
in `src/git/service.rs`,
`app::tests::the_caret_follows_the_file_it_was_on_across_a_refresh` and
`app::tests::a_refreshed_diff_follows_the_same_line_in_the_same_hunk` in
`src/app/tests/git.rs`,
`workspace::host::tests::git_invalidation_is_retained_until_visible_and_fallback_is_not_polling`
and
`workspace::host::tests::a_failed_automatic_refresh_does_not_claim_reconciliation`
in `src/workspace/host.rs`,
`git::tracker::tests::a_narrow_snapshot_preserves_other_staged_bases_and_unrequested_stats`
in `src/git/tracker.rs`,
`app::tests::closing_a_file_retires_its_staged_base` in
`src/app/tests/git.rs`,
`app::tests::save_as_retires_the_previous_paths_staged_base` in that same
file, `app::tests::an_explorer_move_reconciles_git_with_monitoring_disabled`,
`app::tests::a_partial_explorer_report_retries_one_async_post_change_barrier`,
and the settings registry validation tests in `src/settings.rs`.

## Report

The Git status buffer opened with `Space g g` did not refresh automatically.
It was expected to refresh every five seconds by default, with the interval
configurable in the Runyte configuration. The Git branches and Git index
buffers were expected to follow the same refresh behavior.
