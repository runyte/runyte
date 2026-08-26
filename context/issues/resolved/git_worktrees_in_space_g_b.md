---
title: "The branch list does not identify branches checked out in worktrees"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: ed55414
---

## Resolution

Commit `ed55414` (`Show worktree paths in branch list`) fixed the branch-list projection. `GitCliProvider::branches` previously read local refs, current status, merge reachability, and upstream drift but did not associate those branches with the repository's registered worktrees. The `Branch` value consequently had no checkout paths for `branch_rows` to display.

Branch discovery now joins the typed `git worktree list --porcelain -z` results to their full `refs/heads/...` identities inside the existing asynchronous Git worker. Each branch retains every matching checkout as an operating-system `PathBuf`, and `branch_rows` renders each one as a dimmed `[worktree: path]` note while continuing to carry the typed branch separately as the row's action identity. Display conversion is lossy only at the presentation boundary, and control characters are escaped so unusual paths cannot manufacture extra actionable rows. Detached worktrees remain absent from branch annotations because they do not name a local branch; they remain visible in the dedicated worktree list.

The behavior is covered by `git::branch_view::tests::checked_out_branches_show_every_path_without_changing_row_identity` and `git::branch_view::tests::checkout_paths_cannot_manufacture_actionable_rows` in `src/git/branch_view.rs`, plus `typed_worktree_discovery_and_creation_preserve_paths_and_common_identity` and `worktree_discovery_keeps_a_non_utf8_destination_addressable` in `tests/git_provider.rs`. The non-UTF-8 filesystem fixture is ignored on macOS, which rejects that path component with `EILSEQ`; it remains active on Unix filesystems that can represent arbitrary filename bytes.

## Report

The branch buffer opened by `Space g b` showed every local branch, but it did not show which branches were checked out as worktrees or the local directory of each checkout. Checked-out branches should identify their worktrees and show where each one is checked out on the local machine.
