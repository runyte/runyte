---
title: "Branch switching could replace files beneath live terminal jobs"
status: resolved
reported: 2026-08-25
resolved: 2026-08-25
commit: a5288c8
---

## Resolution

Commit a5288c8 (`Confirm branch switches with live terminals`) fixed the branch
switch boundary. `checkout_selected_branch` previously called
`branch_switch_allowed`, which checked only unsaved file buffers before sending
the checkout to Git. It never consulted the workspace-owned terminal sessions,
so pane visibility had no bearing on whether a live child could keep using the
working directory while its files were replaced.

The fix added one prepared `BranchSwitch` path for existing-branch checkout and
branch creation. `request_branch_switch` still applies the existing dirty-buffer
guard, then uses `TerminalSessions::any_live` across the whole workspace. With a
live child it retains the repository and exact checkout or creation action in a
confirmation overlay; the input boundary accepts that action only after the
exact target branch name is typed. Escape or Ctrl-c cancels without submitting
a Git mutation. Exited terminal sessions are not live and therefore leave the
ordinary Enter checkout unchanged.

The report focused on Enter checkout. `Tab n` branch creation was deliberately
included because it also changes the current worktree checkout and presents the
same risk. The worktree list remains unchanged because opening another worktree
switches persistent-session attachment rather than replacing files in the
current workspace.

Regression coverage is in `src/app/tests/git.rs`:
`a_hidden_live_terminal_requires_exact_branch_name_before_checkout` covers a
hidden live child, invalid input, cancellation, exact-name acceptance, and the
ordinary checkout after that child reports its exit;
`creating_a_branch_with_a_live_terminal_requires_exact_name_confirmation`
covers the same safety boundary for `Tab n` creation; and
`confirmed_terminal_branch_checkout_is_submitted_to_the_git_service` covers
the asynchronous production submission path after exact-name acceptance.

## Report

Branch checkout from the branch list was allowed while the current workspace
owned a live terminal session. The terminal job was not terminated by the
checkout, but it kept running in the same working directory while Git replaced
files under it. A watcher, build, test run, or other long-running job could
therefore observe content from two branches or write into the checkout while it
was being changed.

To reproduce:

1. Open a repository in Runyte.
2. Start a terminal with `Space t n` and run a long-lived job in the repository.
3. Leave the terminal visible in another pane or hide it without ending the
   terminal session.
4. Open the local branch list with `Space g b`, select another branch, and press
   `Enter`.

When the repository and Runyte's file buffers were otherwise clean, the branch
checkout proceeded. The presence of the live terminal session was not a
checkout precondition.

Switching branches while the workspace owns any live terminal session requires
an additional safety boundary. A simple Enter confirmation is insufficient
because the branch-list action itself already uses Enter. The boundary must
cover live terminal sessions whether they are visible, hidden, or shown in
another pane, and it must leave checkout unimpeded after the terminal exits.

This issue is limited to changing the branch of the current Git worktree through
the branch list. Opening another worktree with `Space g w` and `Enter` is not the
same operation: in persistent mode it switches the TUI attachment while the old
workspace host retains its buffers and terminal sessions. Worktree attachment
switching remains unchanged.

Implementation was deferred until the adjacent branch-confirmation work on
`enh/confirm-branch-delete` established the shared exact-text confirmation
interaction.
