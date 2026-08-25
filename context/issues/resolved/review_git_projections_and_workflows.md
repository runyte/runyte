---
title: "Git projections and workflows could diverge after renames, external index changes, and incomplete refreshes"
status: resolved
reported: 2026-08-25
resolved: 2026-08-26
commit: cc785f7
---

## Resolution

Commit `cc785f7` (`Harden Git projections and workflows`) resolved the
confirmed projection, correspondence, refresh, and discard problems.

`StatusEntry` previously retained only a rename destination. File-level stage,
unstage, and discard actions could therefore act on one index endpoint and
split a rename, including the unstaged row of an `RM` rename that was modified
again after staging. Status rows now retain both endpoints whenever either
side is genuinely renamed, while copy rows deliberately keep their source
independent. Active-file actions resolve the same correspondence, and the
synchronous path refreshes the staged-text cache for every endpoint it
successfully changed so gutter marks converge with the index even after a
partial failure. Counts for these operations are described as paths rather
than files because a single rename can require two index endpoints.

`App::open_commit_message` previously submitted an asynchronous refresh and
then immediately decided whether anything was staged from the old status
cache. It now attaches commit opening to a successful refreshed snapshot. A
request that coalesced onto an already-running read crosses a second refresh
barrier before opening, while cancellation or failure clears the intent and
cannot be revived by later ambient reconciliation.

`App::apply_git_response` marked the repository snapshot stale when the
snapshot bundled with a completed mutation failed, but did not schedule an
immediate replacement read. It now requests reconciliation at once. Both the
asynchronous and synchronous discard paths also close a clean file buffer when
discard legitimately removes a staged addition or rename destination, rather
than leaving a stale buffer able to recreate the file.

`GitCliProvider::discard` previously used `checkout HEAD`, which could not
discard a path absent from `HEAD`. Staged additions and rename destinations
now use the older, compatible `git rm -f` primitive after explicit index-mode
and no-follow filesystem-shape preflights. Untracked paths, directories, and
gitlinks/submodules are refused before mutation, preventing discard from
recursing into unrelated untracked or ignored descendants. Paths present in
`HEAD` continue to use literal-pathspec `checkout`, retaining compatibility
with Git versions predating `restore`.

Regression coverage is provided by:

- `git::view::tests::a_rename_names_both_ends_and_acts_on_the_new_one`,
  `a_copy_displays_its_source_but_only_acts_on_the_copy`, and
  `both_rows_of_a_renamed_then_modified_file_keep_both_endpoints` in
  `src/git/view.rs`;
- `app::tests::git::rename_rows_submit_both_index_endpoints`,
  `a_copy_row_does_not_unstage_its_independently_changed_source`,
  `active_rename_actions_submit_both_index_endpoints`,
  `synchronous_active_rename_staging_refreshes_both_endpoint_bases`,
  `a_failed_mutation_snapshot_schedules_immediate_reconciliation`,
  `commit_open_waits_for_the_refreshed_index`,
  `cancelling_a_coalesced_commit_check_does_not_reopen_the_intent`, and
  `synchronous_discard_closes_a_removed_staged_addition_buffer` in
  `src/app/tests/git.rs`;
- `discarding_a_staged_addition_removes_it_from_both_trees`,
  `discarding_a_staged_file_refuses_a_replacement_directory_and_keeps_its_children`,
  `discarding_a_staged_gitlink_refuses_to_remove_untracked_submodule_content`,
  and `discarding_both_rename_endpoints_restores_the_original_path` in
  `tests/git_provider.rs`.

The focused application, view, and isolated-provider suites passed alongside
the full repository suite. Independent review covered status, gutter and cache
convergence; stale and coalesced asynchronous results; staging correspondence;
renames, copies and conflicts; branch, worktree, history, blame, stash,
commit, pull and push state; external mutations; and failure recovery. Its
actionable findings were incorporated and the material revisions were
re-reviewed without remaining findings.

Known limitation: Runyte refuses discard for directory and submodule targets
because recursively deciding which nested untracked content may be destroyed
belongs to an explicit filesystem or Git workflow. Repository state can still
change externally between a status projection and a mutation; the mutation's
bundled snapshot and immediate retry on snapshot failure provide convergence,
not a lock against external Git processes.

## Report

Runyte's Git state models, generated views, refresh behavior, and user-facing
Git mutations required a focused hardening review. The lower-level Git
execution boundary had a separate review. This was a proactive review rather
than evidence of a known defect, so changes were limited to confirmed
problems, and every confirmed problem safely within this category required a
fix.

The primary scope was the Git modules outside the execution-boundary focus,
`src/diff.rs`, Git application workflows, and Git tests. The required
invariants covered status and gutter consistency, staged-text caching, refresh
races, stale projections, file, hunk, and selected-line staging, patch
correspondence, aligned diffs, renames, conflicts, branch switching,
worktrees, history, blame, stashes, commit, pull and push state, operation
availability, repository changes made outside Runyte, and recovery after
failed mutations.

Every confirmed defect required isolated-repository regression coverage. All
projections had to converge on actual Git state after both success and
failure.

An independent code review was required after implementation, with the issue,
complete fix diff, relevant invariants, and test results. Every actionable
finding required a fix or technical disposition, and material revisions
required another review. Validation required targeted tests,
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
`cargo test` before resolution.
