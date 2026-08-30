---
title: "Git fixtures use an unexplained `master` branch and an undeclared Git floor"
status: resolved
reported: 2026-08-30
resolved: 2026-08-30
commit: d870473
---

## Resolution

Commit d870473 (`Harden cross-platform integration test boundaries`) changed
`initialize_repository` in `tests/git_provider.rs` to create both ordinary and
bare repositories without Git 2.28's `--initial-branch` option. Initialization
runs with a fixture-local arbitrary `init.defaultBranch`, then explicitly
points `HEAD` at `refs/heads/main` with `git symbolic-ref`. The result is
independent of the person's Git configuration and gives the common fixture the
same `main` branch used by the rest of the provider suite.

Tests that need an unborn branch, detached HEAD, divergence, linked worktrees,
or another topology continue to construct that state locally rather than
inheriting an unexplained `master` default. Bare remote fixtures use the same
initialization rule, so their symbolic HEAD is deterministic as well.

Coverage is provided throughout `tests/git_provider.rs`, including
`an_unborn_branch_still_reports_its_name`,
`branches_report_upstream_drift_and_whether_they_are_merged`, and
`linked_worktrees_share_a_common_repository_identity`. The complete provider
suite passes with 88 tests passed and two platform-specific tests ignored.

Known limitation: the compatibility path uses long-established `git -c` and
`git symbolic-ref` commands, but validation used the installed Git rather than
running an actual pre-2.28 binary.

## Report

`TempRepository::new` in `tests/git_provider.rs` initializes repositories with
`git init -q --initial-branch=master`. Supplying the branch removes dependence
on the machine's `init.defaultBranch`, but `master` differs from Runyte's own
default branch and from the `main` branch used by other fixtures in the same
test file.

The option was introduced in Git 2.28. The project documents a Rust compiler
floor but does not state a minimum supported Git version, so a machine with an
older Git can fail before the behavior under test begins. The failure appears
as a provider-test setup error rather than a clear compatibility decision.

Fixture initialization should use one deterministic branch name, preferably
`main` unless a test specifically exercises another name. The implementation
must also make one of these compatibility choices explicit:

- document Git 2.28 or newer as the supported floor and retain
  `--initial-branch`; or
- initialize in a way that is independent of `init.defaultBranch` without
  requiring that option.

Tests that intentionally require `master`, detached HEAD, unborn branches, or
another topology should declare that locally instead of inheriting it from the
general repository helper. Existing branch, remote, divergence, worktree, and
stash tests must remain deterministic under a user configuration that chooses
an arbitrary default branch.

Reproduction:

1. Configure `init.defaultBranch` to a name other than `master` or `main` and
   run `tests/git_provider.rs`; observe that the current helper still creates
   `master` while later fixtures create `main`.
2. Run the same test with Git older than 2.28; observe setup fail because
   `--initial-branch` is unknown.
