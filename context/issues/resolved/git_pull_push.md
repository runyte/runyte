---
title: "A branch that diverged from its upstream could not be pulled or pushed"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: 0e2ba57
---

## Resolution

Fixed by `0e2ba57`, "Offer to replay local commits when a pull cannot
fast-forward".

The report is accurate and the remembered error was Git's, not Runyte's.
`GitCliProvider::pull` ran `git pull --ff-only --no-rebase --no-stat`, which on
a branch holding commits its upstream does not have prints a block of `hint:`
lines naming `git merge --no-ff` and `git rebase`, then `fatal: Not possible to
fast-forward, aborting.` Runyte reported that verbatim, minus the hints, which
`without_noise` strips.

The refusal itself was deliberate — a merge can stop half-finished and there is
no surface here for resolving one — but it had no way out. `push` was rejected
as non-fast-forward, and the one sentence Git offers about that (`use 'git pull'
before pushing again`) is written on a `hint:` line, so `without_noise` dropped
it too. Pull said the branch could not fast-forward, push said to pull first,
and nothing in the editor could move the branch.

Divergence is now `GitError::Diverged`, carrying the branch, the upstream, and
the two counts, rather than a `Failed` holding whatever `--ff-only` printed. The
`--ff-only` attempt has already fetched by the time it refuses, so
`GitCliProvider::divergence` reads the drift back from the refs Git now holds
rather than inferring it from the wording of the failure; only a branch that has
moved both ways is reported this way, and every other pull failure keeps the
message Git wrote for it.

`p` turns that error into a confirmation — `main and origin/main have both moved
on. Press Enter to replay 2 local commits on top of the 1 on origin/main` —
where Enter runs the new `GitProvider::rebase_onto_upstream` and Escape leaves
the branch alone. It is a confirmation rather than an automatic reconcile
because a fast-forward decides nothing while a rebase rewrites commits, which
survive only in the reflog under their old identities. Both the synchronous
provider path and the asynchronous service path route through
`App::report_pull_failure`, so the offer appears wherever the failure arrives
from; an outcome the service reports as uncertain stays an error, because there
is then no state safe to propose replaying onto.

Rebase rather than merge, deviating from what the report left open: the
fast-forward-only policy already implies this history stays linear, and a rebase
produces no merge commit whose message Runyte would have to invent or prompt
for. The implementation runs `git pull --rebase` rather than
`git rebase @{upstream}` so the commits land on what the remote holds now rather
than on a tip that may already be stale.

The invariant `--ff-only` protected holds for the replay too.
`GitCliProvider::rebase_onto_upstream` checks `rebase_in_progress` after a
failure — via `git rev-parse --git-path`, because rebase state belongs to one
worktree rather than to the repository — and runs `git rebase --abort` when a
replay stopped partway, so a conflict leaves the working tree exactly as it was
rather than holding markers nothing here can resolve.

Three things about that invariant were wrong in the first implementation and
were fixed in review, each with a regression test that fails without the fix.

Both commands now pass `--no-autostash`. With `merge.autoStash` or
`rebase.autoStash` configured, Git stashes uncommitted changes, does the work,
and reapplies them; when the reapplication conflicts it prints an explanation
and *still exits zero*. The working tree is left holding conflict markers and a
stash, and because nothing is mid-rebase afterwards the rollback above never
ran — so the provider reported success over a conflicted tree, which is exactly
what the invariant promises cannot happen. Refusing a dirty worktree up front is
the outcome a reader can act on. This affected the pre-existing `--ff-only` pull
as much as the new replay.

The rollback runs on `GitCliProvider::uncancellable`, a clone with the
cancellation flag dropped. `:git-cancel` stops a command by setting a flag the
provider checks before every wait, and that flag is still set while the cleanup
runs — so the probe would have been refused and the abort killed, leaving the
half-finished tree that cancelling was meant to spare the reader. Cancellation
is not rollback elsewhere in the Git layer because nothing else knows how to
roll back; a stopped rebase does. A cancelled replay still returns its own
`Cancelled` error so the service's bookkeeping is unchanged; what changed is
that the tree it reconciles is the one the reader started with. An abort that
itself fails is now reported rather than discarded.

The fetch and the merge are separate commands rather than one `git pull`.
Divergence is read from the remote-tracking refs, and those outlive the
connection that filled them: a branch that was already diverged still looks
diverged when the remote is unreachable. A single failed `pull` says nothing
about whether its fetch got that far, so an offline remote or a failed
authentication was being reported as drift and answered with an offer to replay
commits onto a tip nobody had seen. Fetching first means the network error is
reported as itself and the drift is only ever read from refs a fetch actually
refreshed. `git merge --ff-only @{upstream}` replaces `git pull --ff-only`,
which also makes the status line lead with `Updating abc..def` rather than with
the `From <remote>` header.

Two things around the edges came with it. A rejected push now names catching up
itself, since the `hint:` line that said so is filtered out with the rest of the
noise; `rejected_as_stale` matches both wordings Git uses, `non-fast-forward`
when the local tip is simply behind and `fetch first` when the clone has not
fetched since the remote moved. And network summaries pass through `settled`,
which keeps only what follows the last carriage return on each line, so
`Rebasing (1/2)` and `Receiving objects: 40%` no longer run into the sentence
after them in the status row.

Covered by, in `tests/git_provider.rs`:
`pulling_fast_forwards_and_refuses_a_branch_that_has_diverged` (the refusal is
`Diverged` with its counts and moves nothing),
`rebasing_replays_local_commits_onto_the_upstream` (the collaboration case
end to end: two local commits, one remote, both sets of work present afterwards,
history linear, and the push that was rejected before now goes through),
`a_conflicting_rebase_is_undone_and_changes_nothing`,
`rebasing_needs_a_branch_with_an_upstream`,
`a_diverged_pull_and_its_replay_run_through_the_ordered_async_service`,
`pushing_refuses_rather_than_overwriting_the_remote` for the catch-up sentence,
`a_configured_autostash_cannot_turn_a_refusal_into_a_conflicted_success`, and
`an_unreachable_remote_is_reported_as_itself_and_not_as_stale_drift`.
In `src/app.rs`:
`pulling_a_diverged_branch_offers_to_replay_the_local_commits` and
`a_diverged_pull_from_the_git_service_opens_the_offer_rather_than_an_error`. In
`src/git/cli.rs`: `progress_counters_settle_to_what_the_terminal_would_show`
and `cleanup_after_a_cancellation_still_runs_git`.

Known limitation: a replay that conflicts still has to be finished with Git
outside Runyte, which has no conflict-resolution surface. The offer is only made
for the branch the working tree is on, so a diverged branch that is not checked
out is still reported as a plain push rejection. Force-pushing remains
unavailable by design, so a branch whose upstream was rewritten by someone else
cannot be reconciled here at all.

`cleanup_after_a_cancellation_still_runs_git` covers the mechanism the rollback
depends on — that dropping the flag reaches Git while the original provider
still refuses — rather than the end-to-end race it protects against. Cancelling
at the exact moment a replay stops on a conflict is not reproducible from a
test, because it needs the flag to be set after `upstream_branch` has read the
branch and before the rebase finishes. The two halves are each covered; the
timing between them is not.

## Report

`git pull` and `git push` appear to have bugs in the case where two or more
people collaborate and push commits to the same branch — for example, having N
commits locally that are not pushed to the remote while the remote also holds K
commits that are not present locally.

`git pull` (the `p` command) failed in such a situation with an error about
`git rebase`; the exact wording was not recorded.

The report asks for thorough testing of these collaboration cases. It does not
say what the pull should do instead of failing.
