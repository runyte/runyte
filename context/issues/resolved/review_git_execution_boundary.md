---
title: "Git execution boundaries did not consistently contain hostile paths, configuration, output, and descendants"
status: resolved
reported: 2026-08-25
resolved: 2026-08-25
commit: 9d23677
---

## Resolution

Commit `9d23677` (`Harden Git execution boundaries`) resolved the confirmed
problems in the Git command-line provider and repository lock.

`GitCliProvider` previously stopped reading stderr at its retention limit.
Closing that pipe could send SIGPIPE to an otherwise successful verbose hook
and change the command's result. The bounded readers now drain without
retaining bytes past their limit, signal stdout overflow to the parent, and
stop the owned process group explicitly. Reader completion after top-level Git
exit is bounded as well: a descendant that retains inherited pipes is stopped
with the process group where possible, while a descendant that escaped that
group produces a typed I/O failure without holding the worker or repository
lock in an unbounded join.

`GitCliProvider::command` now removes inherited repository, index, object,
one-shot configuration, executable-helper, and template authority, including
indexed `GIT_CONFIG_KEY_*` and `GIT_CONFIG_VALUE_*` entries that could
otherwise reach hooks. It retains argv-only execution and the existing prompt
policy. Executable spawn failures now consistently report
`GitError::Unavailable`; an invalid working directory remains an I/O error so
discovery does not confuse it with a missing Git installation.

Path arguments were protected from option parsing with `--`, but Git pathspec
magic was still active. Direct pathspec-consuming commands now use Git's
literal-pathspec global option, so a filename such as `:(glob)*` cannot widen
a one-file mutation. The option is deliberately scoped to commands that
consume Runyte-supplied paths rather than every Git child, because applying it
globally also changes internal pathspecs used by commands such as `git stash`.
Repository-relative validation now rejects parent traversal, blame uses that
same boundary, and local reads reject regular files reached through a
symlinked parent outside the worktree.

`GitCliProvider::apply_partial` previously checked the repository fingerprint,
disk hash, hunk identity, and patch applicability without proving that the
patch bytes belonged to the request's claimed path. It now re-reads the exact
current diff for that path and scope and requires a byte-identical identified
hunk before either `git apply --check` or the mutation. `parse_hunks` also
requires a line-start `diff --git` file header. A crafted request can no longer
name one identical file while applying another file's patch.

Finally, `repository_lock::RepositoryGuard::drop` and cancelled-reservation
cleanup now remove inactive, empty repository entries. FIFO ordering and
same-thread reentrancy are unchanged without retaining one map entry for every
repository seen during a long-lived process.

Regression coverage is provided by:

- `git::cli::tests::long_failure_logs_are_large_and_explicitly_bounded`,
  `oversized_stdout_is_signalled_after_the_retained_bound`,
  `a_detached_helper_cannot_hold_completed_command_pipes_open`,
  `a_session_escaping_helper_fails_without_blocking_the_worker`,
  `inherited_git_authority_is_removed_from_every_command`,
  `local_reads_refuse_traversal_and_symlinked_parent_escapes`, and
  `a_non_executable_git_reads_as_unavailable` in `src/git/cli.rs`;
- `git::patch::tests::a_non_git_diff_header_is_not_an_applicable_patch` in
  `src/git/patch.rs`;
- `git::repository_lock::tests::an_idle_repository_does_not_leave_lock_state_behind`
  in `src/git/repository_lock.rs`;
- `a_successful_verbose_hook_is_not_killed_by_the_error_bound`,
  `pathspec_magic_in_a_filename_never_broadens_staging`,
  `blame_refuses_a_parent_traversal_before_git_sees_it`, and
  `partial_patch_bytes_are_bound_to_the_claimed_path_at_apply` in
  `tests/git_provider.rs`.

Existing isolated-repository coverage for option-shaped remotes, awkward shell
characters, non-UTF-8 paths, output limits, cancellation, network deadlines,
repository concurrency, and partial-stage staleness was run with these tests.

Known limitation: a descendant that starts a new session cannot be killed
portably through Git's original process-group ID. Runyte now bounds its wait
and reports the failure, but a detached reader thread remains until that
descendant closes the inherited pipe. External processes also do not
participate in Runyte's repository lock, leaving an irreducible interval
between the last partial-stage precondition check and `git apply`; Git still
checks patch applicability. Canonical containment followed by a pathname open
retains the usual symlink-swap race; closing it portably requires
descriptor-relative, no-follow traversal beyond this local Git-boundary fix.

## Report

The boundary that invokes Git and converts its output into bounded structured
results required a focused hardening review. Repository contents, refs, paths,
configuration, hooks, and Git output are untrusted inputs. The review was
proactive rather than based on one previously known defect, and changes were
limited to confirmed problems.

The primary boundary is `src/git/cli.rs`, `service.rs`, `patch.rs`,
`repository_lock.rs`, and their provider tests. The required invariants cover
argument construction and option termination, revision and path ambiguity,
hostile names, output and error bounds, invalid encodings, process and
credential-prompt behavior, environment inheritance, cancellation, timeouts,
child cleanup, repository locking, concurrent operations, patch validation,
path containment, and failure classification. Commands remain argument
vectors and never pass through a shell.

The review confirmed that stderr retention could alter hook behavior, escaped
descendants could hold reader joins indefinitely, Git pathspec magic could
broaden literal filename actions, inherited one-shot Git configuration could
retarget children or leak into hooks, and executable-start failures could be
silently classified as no repository. It also confirmed inconsistent parent
traversal checks, local-read escape through symlinked parents, a missing
claimed-path binding for partial patches, acceptance of non-Git diff headers,
and unbounded growth in idle repository-lock bookkeeping.

Every confirmed defect required isolated-repository or subprocess regression
coverage where applicable. An independent code review was required after the
implementation, with the complete diff, these invariants, and test results.
Every actionable review finding had to be addressed, and material revisions
had to be reviewed again. Validation required targeted tests,
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
`cargo test` before resolution.
