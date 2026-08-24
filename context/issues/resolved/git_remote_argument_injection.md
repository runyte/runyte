---
title: "Git push treated repository-configured remote names as possible options"
status: resolved
reported: 2026-08-21
resolved: 2026-08-21
legacy_commit: 2ea519a
---

## Resolution

Commit 2ea519a (`Guard Git push remote arguments`) fixed this. `GitCliProvider::push` was constructing its argument vector directly from the remote name and remote ref exposed by `branches` or `default_remote`, without either validating those repository-configured values or ending Git's option parsing before the remote. The existing branch operations guarded the same boundary, but push did not.

`validate_push_destination` now rejects remote names and remote refs beginning with `-` at the point where push consumes them. The validation deliberately does not happen while reading branches, so an option-shaped configured upstream remains visible for inspection and only the unsafe operation is refused. Every push argument vector also places `--` after Runyte's own options and before the remote as a second, independent guard against Git interpreting it as an option.

Tests covering the behavior are:

- `pushing_refuses_an_option_shaped_default_remote` in `tests/git_provider.rs`
- `pushing_refuses_an_option_shaped_tracked_remote` in `tests/git_provider.rs`
- `option_shaped_remote_refs_are_not_push_destinations` in `src/git/cli.rs`
- `pushing_publishes_a_tracked_branch_and_adopts_an_untracked_one` in `tests/git_provider.rs`

## Report

Git subprocess arguments in `src/git/cli.rs` guarded branch names and file paths against being read as options, but remote names and remote refs were passed to `git` without the same protection.

`checkout_branch`, `create_branch`, `delete_branch`, and the branch-history path rejected values that begin with `-` and/or inserted a `--` separator before positional arguments, so a branch called `--foo` could not be mistaken for a `git` option. `push` and `default_remote` did not do this for the remote name or the upstream ref:

- `push` built `["push", <remote>, "<branch>:<reference>"]`, `["push", "--set-upstream", <remote>, "<branch>:<reference>"]`, or `["push", "--set-upstream", <default_remote>, <branch>]`.
- `default_remote` returned a name taken directly from `git remote`.

None of these values were typed by the user. `<remote>` and `<reference>` came from `.git/config` — `branch.<name>.remote` and `branch.<name>.merge`, read through `%(upstream:remotename)` / `%(upstream:remoteref)` in `branches`, or from `git remote`. A repository whose `.git/config` named a remote such as `--receive-pack=<command>` (equivalently `--exec=` or `--upload-pack=`) would have that string handed to `git` as an option rather than as a value. `.git/config` is not carried by `git clone`, but it is present when a repository is delivered as an archive, on a shared or removable filesystem, or otherwise copied rather than cloned, so the value was attacker-influenced for a repository the user did not create.

With the argument order at the time, the injected option was consumed and the next positional argument (a refspec or branch name) became the repository Git tried to resolve, which then failed. A reliable command-execution path was therefore not demonstrated from `push` as written. The problem was the inconsistent boundary: the same class of input was guarded for branch names and left open for remotes, so a later argument-order or refspec change could have turned the latent gap into a live one. `<reference>` reaching the refspec unguarded was part of the same gap.

The required behavior was to mirror the existing branch handling by rejecting remote names and refs that begin with `-`, placing a `--` separator before positional push arguments, or both, so a value from `.git/config` could never be read as a Git option regardless of argument order. Whether rejection should happen while reading `branches` / `default_remote` or at each Git call site, and the error shown to the user, were initially undecided.
