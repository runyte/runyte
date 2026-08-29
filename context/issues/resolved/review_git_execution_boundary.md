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
  `a_session_escaping_helper_cannot_hold_the_completed_worker`,
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
The non-UTF-8 filesystem fixtures remain active on Unix filesystems that can
represent arbitrary filename bytes and are ignored on macOS, which rejects
their setup with `EILSEQ`.

A 2026-08-29 follow-up corrected two related assumptions exposed by lifecycle
stress coverage on loaded macOS runners. Reader completion had still been
decided by whether the stdout and stderr threads were scheduled within a 50 ms
grace period after a successful child exit. Unix pipe workers now wait in the
kernel for pipe readiness or a private finalizer wake and finish from
observable pipe EOF or the explicit top-level completion boundary, so thread
scheduling cannot turn a completed `rev-parse` into an I/O error. Descendants
in Git's process group are signalled and their queued output is drained; a
process that escaped the group while retaining a pipe cannot delay the
completed Git result or retain a reader thread: after
the top-level exit, Runyte signals the former process group once, keeps all
bytes already available, and closes the stream at `EAGAIN`. It does not poll
`kill(-pgid, 0)`: once the group leader has been reaped, that numeric identifier
is not an ownership-safe completion event and can remain visible or be reused.
This cutoff is necessary because an escaped writer and a CLOEXEC descriptor
temporarily inherited by a concurrent fork are indistinguishable from pipe
state alone. Input-writer failures are joined and propagated rather than
discarded. `GitCliProvider::discover` also
identifies ordinary repository absence from `.git` ancestry before starting
Git. A directory marker requires a regular `HEAD`, and a file marker requires
a bounded, nonempty `gitdir: ` target; malformed and unreadable markers produce
typed I/O errors. Once a valid marker exists, all `rev-parse` errors propagate
instead of being converted into a cached `None` result. A strict descendant of
the canonical system temporary directory, or of another sticky world-writable
directory, stops discovery before that shared ancestor: a `.git` entry in a
shared scratch root cannot claim or poison every private workspace below it.
Markers below that ceiling remain decisive, including malformed markers.

A later 2026-08-29 follow-up removed the remaining identity and classification
ambiguity. Successful Unix commands are now observed with
`waitid(WEXITED | WNOHANG | WNOWAIT)`: the exited leader remains waitable while
Runyte stops its process group, so cleanup cannot signal a newly reused group
number. Only then is the leader reaped and the final pipe drain released.
Repository discovery treats empty `--show-toplevel`, `--git-dir`, and
`--git-common-dir` results as malformed output once the marker probe succeeds;
marker absence is the only ordinary `None` result. The application also keeps
the discovery error distinct from completion-without-a-repository, so command
availability and protocol frames report a failed Git discovery instead of
mislabeling it or waiting indefinitely for a capability that cannot appear.

A later lifecycle-gate follow-up made that identity-preserving completion
platform-specific. Darwin's `waitid(WNOWAIT)` path could fail to publish a
completed child under load, leaving repository discovery blocked while the
editor continued producing frames. Git children on macOS now receive an
`EVFILT_PROC`/`NOTE_EXIT` kqueue registration immediately after spawn. The
knote reports exit without reaping the leader, so Runyte can stop the process
group while its identity is still anchored and collect the status afterward.
Other Unix targets retain `waitid(WNOWAIT)`. The pipe wait also gives the
explicit finalizer wake priority over simultaneous data readiness, preventing
Darwin's poll adapter from repeatedly selecting stale EOF readiness instead
of beginning the final nonblocking drain. Child completion is published before
test-gated pipe workers are released, and an input writer rejects remaining
bytes after that boundary. A concurrently forked process may briefly inherit a
close-on-exec pipe descriptor, but its kernel reader can no longer make input
look consumed by a Git child that has already exited. Output readers still use
the same boundary to perform one final nonblocking drain.

The initial Darwin observer still had a spawn-to-registration gap for commands
that exited immediately. XNU's process filter reports edges after attachment;
it does not replay an earlier `NOTE_EXIT` merely because the unreaped zombie
still accepts a knote. The observer now installs the knote first and immediately
queries `PROC_PIDTBSDINFO` without reaping. A zombie snapshot records the
already-completed child, while a live snapshot leaves every subsequent exit
covered by the installed knote. In both cases the leader remains waitable until
Runyte has stopped its still-anchored process group.

Follow-up regression coverage is provided by
`git::cli::tests::fast_output_survives_readers_held_until_after_child_exit`,
`finalizer_wake_is_followed_by_a_fresh_eof_read`,
`failed_input_write_cannot_be_reported_as_git_success`,
`discovery_without_a_marker_does_not_invoke_git`,
`discovery_with_a_marker_propagates_git_failure`,
`repository_marker_probe_distinguishes_absent_present_and_invalid`, and
`shared_scratch_marker_is_a_ceiling_but_private_markers_remain_decisive` in
`src/git/cli.rs`, together with
`repository_discovery_rejects_empty_required_rev_parse_output` in
`src/git/cli.rs`,
`finalizer_wake_wins_when_pipe_data_is_ready_too` and the macOS-only
`darwin_child_exit_observer_covers_registration_after_exit` in `src/git/cli.rs`,
`git_project_availability_distinguishes_missing_git_and_non_repository` in
`src/app/tests/language.rs`,
`linked_worktrees_share_a_common_repository_identity` in `tests/git_provider.rs`
and the semantic Git-discovery barrier in
`attaching_with_logging_flags_reports_the_retained_configuration` in
`tests/diagnostic_log.rs`.

Known limitation: a descendant that starts a new session cannot be killed
portably through Git's original process-group ID. On Unix, it cannot extend
the worker after the top-level command completes, but output it writes later is
outside the completed command's retained result. Runyte requests termination
of helpers in the original group without synchronously proving that every
descendant has exited; doing that portably requires a process supervisor or
identity-bearing platform handles. Other platforms retain the bounded
reader-grace behavior. External processes also do not participate in Runyte's
repository lock, leaving an irreducible interval
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
