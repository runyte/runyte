---
title: "Automatic Git refresh polled unchanged repositories"
status: resolved
reported: 2026-08-29
resolved: 2026-08-29
commit: 30e093c
---

## Resolution

Commit `30e093c` (`Make automatic Git refresh event-driven`) resolved this
report.

`WorkspaceHost::refresh_git_if_due` had treated
`git.refresh_interval_seconds` as the primary refresh trigger, and
`App::git_refresh_spec` requested staged content for hidden open files. That
made an ordinary visible tracked file run Git repeatedly even when neither the
checkout nor its metadata had changed.

The new `git_monitor` owns a bounded native-observation queue, watches the
checkout plus its worktree-private and shared Git directories, ignores
read-only access events, and turns each burst after a 150 ms quiet period into
one repository invalidation. Watcher errors and queue overflow deliberately
become full invalidations; events remain hints, and only the Git service reads
authoritative repository state. Repository discovery now retains both
`--git-dir` and `--git-common-dir`, covering ordinary repositories, linked
worktrees, and Git-directory indirection.

`WorkspaceHost::refresh_git_if_due` now retains dirty state until a Git
consumer is visible, accumulates the `RefreshSpec` requirements already
reconciled for that invalidation, and submits only the current visible panes'
requirements. The configured interval is a maximum-staleness fallback instead
of routine polling. Its measured default is 60 seconds, while zero unregisters
the watcher and disables both automatic paths. Explicit refresh and snapshots
bundled with editor-owned mutations keep their direct paths and the Git
service's existing generation and mutation ordering. A fixed 250 ms
interaction quiet period preserves prompt, search, deliberate-selection, and
row-identity protections without delaying an observed change for the full
fallback interval.

A review follow-up made partial snapshots explicitly self-describing and
completion-aware. `GitTracker::apply_snapshot` now replaces only staged bases
and statistics the snapshot actually requested, so revealing file B cannot
evict file A's already reconciled gutter base. Terminal-covered buffers are
not visible Git consumers. The host records coverage only after a successful,
non-coalesced snapshot. Failed automatic reads keep the repository dirty and
retry after a bounded delay rather than claiming freshness until the fallback
interval.

A second review follow-up moved that freshness barrier to immediately before
the first Git read and timestamps native observations in the watcher callback
before they enter the bounded command queue. An observation made during a
multi-field refresh therefore cannot be mistaken for state the snapshot read,
while an earlier observation remains safe to absorb even if queueing delays its
delivery. Closing the last buffer for a file now retires its staged base, and
late direct or snapshot responses cannot recreate that unreferenced cache
entry. Monitor shutdown uses an atomic stop request in addition to the bounded
command channel, so a saturated queue cannot keep the worker and its callback
sender alive.

A further queue-ordering refinement keeps observation time monotonic across
queued events and overflow, while measuring the debounce quiet period from
worker receipt rather than callback time. Snapshot coverage now requires an
observation to be strictly earlier than the first-read barrier, avoiding an
ambiguous equal-timestamp edge. Partial status and staged-content reads no
longer clear repository-wide stale state, because they cannot prove they cover
an invalidation. Save-as also retires the previous path's staged base before
tracking the new path.

Confirmed explorer filesystem plans also reconcile Git directly after every
successfully applied portion, including partial failures. File and directory
moves retire each retargeted file buffer's old staged base and track its new
path, so editor-owned filesystem changes neither depend on watcher delivery
nor leak old keys when automatic monitoring is disabled.
Asynchronous plan reconciliation is a distinct non-coalescing service read,
ordered after every read that could have started before the filesystem
change. Its staged paths are submitted as one snapshot rather than one request
per moved buffer. If the bounded service queue is full, the editor retains and
conflates the required snapshot until capacity is available; that retry runs
even when automatic monitoring is disabled.

Tests: `git_monitor::tests::a_native_burst_produces_one_debounced_invalidation`,
`git_monitor::tests::linked_worktree_watches_checkout_private_and_shared_metadata`,
and `git_monitor::tests::worktree_index_head_refs_and_packed_refs_are_relevant`
in `src/git_monitor.rs` cover native coalescing and metadata scope;
`workspace::host::tests::git_invalidation_is_retained_until_visible_and_fallback_is_not_polling`
in `src/workspace/host.rs` covers the visibility gate, retained dirty state,
narrow requirements, fallback, and zero setting;
`workspace::host::tests::a_snapshot_covers_only_observations_before_its_first_read`
and
`workspace::host::tests::a_failed_automatic_refresh_does_not_claim_reconciliation`
cover the freshness boundary, direct mutation reconciliation, and
completion-aware failures;
`git::tracker::tests::a_narrow_snapshot_preserves_other_staged_bases_and_unrequested_stats`
in `src/git/tracker.rs` covers non-destructive partial snapshots;
`git_monitor::tests::dropping_the_handle_requests_shutdown_even_when_the_queue_is_full`
and
`git_monitor::tests::queued_observations_never_regress_freshness_or_expire_the_debounce`
in `src/git_monitor.rs` cover bounded-queue shutdown, monotonic observation
time, and delayed-queue debouncing;
`git::service::tests::post_change_reconciliation_does_not_join_an_active_refresh`
in `src/git/service.rs` covers the post-filesystem ordering barrier;
`app::tests::async_refresh_requests_staged_bases_only_for_visible_open_files`
and `app::tests::closing_a_file_retires_its_staged_base`, together with
`app::tests::save_as_retires_the_previous_paths_staged_base` and
`app::tests::an_explorer_move_reconciles_git_with_monitoring_disabled`, plus
`app::tests::a_partial_explorer_report_retries_one_async_post_change_barrier`
and
`app::tests::automatic_refresh_waits_out_a_short_quiet_period_after_the_last_keystroke`
in `src/app/tests/git.rs` cover visible, maximized, terminal-covered panes,
closed-buffer and save-as cache retirement, partial and late responses, and
interaction;
`config::tests::git_reconciliation_defaults_to_sixty_seconds` in
`src/config.rs` covers the new default; and discovery coverage in
`tests/git_provider.rs` verifies main, linked-worktree, and separate Git
directories. The unchanged-repository, 100-open-buffer, and external burst
subprocess measurements are recorded in
`context/reference/startup-performance.md`.

Known limitation: native watcher support and event delivery remain
platform/filesystem dependent. Conservative repository-wide invalidation and
the configured periodic fallback recover from unclassified or lost events;
the watcher does not attempt to infer Git status from event paths.

## Report

Runyte currently uses `git.refresh_interval_seconds` as the primary trigger for
automatic Git refresh. `WorkspaceHost::refresh_git_if_due` checks the timer and
submits a background refresh only while `App::has_visible_git_state` reports a
visible consumer. A visible consumer is either a Git-derived projection such
as the changed-file list opened by `Space g g`, or an ordinary tracked file
whose gutter depends on cached Git state.

The visibility gate prevents periodic Git subprocesses while only terminals,
scratch buffers, or files outside the repository are visible. During ordinary
editing, however, a tracked file is normally visible, so Runyte executes a
repository status refresh after every configured interval even when neither
the worktree nor the repository metadata changed. Each refresh reads status
and `HEAD`; its `RefreshSpec` may also request status statistics and other
projection data, and currently reloads staged content for every open tracked
file rather than only the files with visible gutters.

The Git service keeps this work off the input and rendering thread and
coalesces equivalent queued reads, but the refresh trigger remains polling
rather than invalidation-driven.

## Expected behavior

Filesystem and repository-metadata observations should be the primary trigger
for automatic Git refresh. A relevant change should mark the affected
repository state dirty. Runyte should debounce a burst of observations until
it settles, coalesce duplicate and overlapping invalidations into one bounded
refresh request, and request only the data required by the current visible
consumers.

Relevant observations include changes in the worktree and changes made by
external Git operations to the index, `HEAD`, refs, packed refs, stashes, and
worktree-specific metadata. Repository discovery must account for ordinary
repositories, linked worktrees, and Git-directory indirection rather than
assuming that all metadata lives under `<workspace>/.git`.

If no Git consumer is visible, an observation should not immediately run Git.
It should retain enough dirty state for the next projection or tracked-file
gutter that becomes visible to request a current snapshot. Opening or
revealing a dirty `Space g ...` projection should refresh it without waiting
for the periodic interval. Revealing a tracked file with a dirty gutter base
should likewise refresh the data required for that file; hidden file buffers
do not need their staged content reloaded merely because another Git consumer
is visible.

The timer should remain as a reconciliation fallback for lost filesystem
events, unsupported watcher behavior, remote or unusual filesystems, and Git
operations whose observable writes were not classified correctly. After this
change, `git.refresh_interval_seconds` is the maximum-staleness interval for a
visible Git consumer rather than the routine refresh mechanism. Its default
should become materially longer than the current five-second polling default;
the exact fallback cadence should be selected with correctness and subprocess
measurements rather than copied from the old polling design. An explicitly
configured value remains the requested fallback cadence.

A value of `0` continues to disable watcher-triggered and fallback refreshes;
`Space g r` and `:git-refresh` remain available, and reconciliation required
after an editor-owned Git mutation still occurs as part of that mutation.

Editor-owned Git mutations and file operations should continue to request the
known reconciliation they require directly. They must not wait for the
filesystem watcher to report the writes Runyte itself initiated.

## Debouncing and coalescing

A save, checkout, rebase, or staging operation can produce a rapid sequence of
create, write, rename, lock-file, index, and ref observations. These are hints
that cached state may be stale, not independent commands to run Git.

Debouncing should wait for a short quiet period after the latest related
observation so one filesystem operation does not cause several snapshots of
its intermediate states. Coalescing should union the pending repository,
path, and projection requirements, remove duplicates, and submit one refresh
whose result satisfies every still-relevant consumer. A later, broader request
may subsume an earlier narrow request, but Git mutations retain their existing
ordering and must not be crossed by a read in a way that publishes a snapshot
from the wrong side of the mutation.

The refresh result remains authoritative. Watcher observations must not be
interpreted as Git status themselves, and a missing, duplicated, reordered, or
overflowed event must not corrupt the cached snapshot.

## Interaction and failure constraints

- All Git execution remains inside `src/git/`; watcher callbacks and the host
  thread do not run Git or perform unbounded work.
- Observation and refresh queues remain bounded. Overflow marks the repository
  dirty for full reconciliation instead of accumulating an unbounded path
  list.
- Existing protection for command input, searches, deliberate selections, and
  row identities in refreshed projections remains in force. Event-driven
  invalidation must not make a projection rewrite more disruptive.
- Refresh requests and results remain generation-aware so a result for an old
  repository or workspace cannot update current state.
- A watcher failure or an automatic refresh failure retains the last good
  snapshot and its stale indication. The periodic fallback must remain capable
  of recovery.
- The setting description and user guide must explain the event-driven path,
  fallback interval, visibility gate, and meaning of zero.

## Reproduction and measurement

Open a tracked file in a repository, configure a long fallback interval, leave
the worktree and Git metadata unchanged, and observe Git subprocess activity
for longer than the former five-second polling cadence but less than the
configured fallback. Runyte currently performs repeated status refreshes
because the file's possible gutter is a visible Git consumer. After this
change, the unchanged repository should produce no Git subprocess after the
initial snapshot during that window. A relevant observation should cause one
debounced refresh, and expiry of the configured fallback should still cause
reconciliation when no observation arrived.

Performance verification should cover both a small repository and a
repository with many open buffers. It should distinguish cheap host timer
checks from actual Git subprocesses and record how many refresh requests and
subprocesses a burst of filesystem events produces.

## Regression coverage

Tests should cover:

- no automatic Git request when no Git consumer is visible;
- no routine subprocess before fallback reconciliation in an unchanged
  repository with a tracked file visible, together with coverage for the new
  longer default;
- one debounced, coalesced refresh for a burst containing duplicate worktree,
  index, and ref observations;
- an external worktree change updating the visible changed-file list;
- an external index or `HEAD` change updating visible status, branch data, and
  tracked-file gutters as applicable;
- invalidation retained while no consumer is visible and reconciled promptly
  when a Git projection or tracked file becomes visible;
- hidden tracked buffers excluded from staged-content reads until their gutter
  becomes visible;
- a simulated lost or overflowed observation recovered by the periodic
  fallback;
- `git.refresh_interval_seconds: 0` disabling event-triggered and fallback
  refresh while explicit refresh remains functional;
- editor-owned Git mutations refreshing directly without waiting for watcher
  delivery;
- refresh reads remaining correctly ordered with Git mutations; and
- command input, search matches, deliberate selections, row identity, and
  stale-result generation guards surviving event-triggered refreshes.
