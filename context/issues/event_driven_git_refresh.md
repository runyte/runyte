# Git refresh should be event-driven with periodic reconciliation

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
