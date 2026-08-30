# A Git discovery child is terminated by SIGKILL on macOS

Repository discovery intermittently fails on macOS with
`` `git rev-parse --show-toplevel` failed after termination by signal 9 ``.
The capability that depends on it never becomes available, so any test
waiting for a Git-only command row times out on the CI macOS runners. The
failure has appeared in both the `Tests (macos-latest)` and
`Lifecycle stress (macos-latest)` jobs, in
`creating_a_worktree_starts_and_attaches_its_persistent_session`,
`persistent_worktree_switch_detaches_to_a_new_root_without_retargeting_the_host`,
and `incompatible_worktree_host_returns_the_tui_to_its_source`.

## What was suspected

A negative PID names a process group, and the kernel recycles that number once
the group is empty and its leader reaped. Every subsystem that spawns a child
into its own group can therefore reap the child and then signal a number that
belongs to something else. At the time, every Git child called `setsid`, so its
process group identifier was exactly its own PID, which is the shape a recycled
number would take. Three real defects of that class were found and corrected:

- `b09c444` — PTY teardown signalled `-pid` after `Pty::finished` had reaped
  the terminal leader through `Child::try_wait`.
- `7f7e8c0` — Git's `stop_child_tree` signalled `-pid` on paths that run after
  `try_finish_child` has already reaped the leader.
- `8dadcc6` — clipboard cleanup signalled `-child.id()` after `wait_until` had
  reaped the helper through `Child::try_wait`.

None of them explains this failure.

## What the audit established

`src/process_group.rs` centralizes every production negative-process-group
signal behind an ownership proof and, when `RUNYTE_PROCESS_AUDIT` names a
file, records each signal with its sender, signed target, subsystem and call
site, owned child, whether that child is running or reaped, and the `getpgid`
and `getsid` the kernel reports for it. Git child spawns and authoritative
`waitid` completions are recorded alongside. Only the local-protocol suite
sets the variable; ordinary application use writes nothing.

Run 33278657812 reproduced the failure with that journal in place. The records
naming the child that died read, in order:

```text
event=spawn      child_pid=20746 command=git rev-parse --show-toplevel getpgid=-1 getsid=-1
event=completion child_pid=20746 source=darwin_waitid_wnowait code=None signal=Some(9)
event=signal     child_pid=20746 target=-20746 signal=9 child_state=unreaped_leader outcome=sent
```

Two facts follow.

No Runyte process signalled that child before it died. The only signal record
naming it is its own cleanup, which stands after the completion that had
already classified it as terminated by signal 9, and that status is captured
by `waitid(WNOWAIT)` before any cleanup runs. Across the whole journal every
delivered group signal came from Git's own `try_finish_child`, aimed at its
own still-unreaped child; no clipboard, PTY, or `stop_child_tree` signal
appeared at all, and no signal named a group its sender did not own.

The child was already gone when `Command::spawn` returned. Its spawn record
reports `getpgid=-1 getsid=-1`, while every sibling spawn in the same journal
reports the child's own identifier for both. XNU's process lookup does not
report a zombie, so `-1` at that point means the child could no longer be
found. It was terminated at or immediately after `execve`, before it ran.

The same journal shows Darwin answering `getpgid` and `getsid` with `-1` for
any completed but unreaped child, where Linux answers with the child's own
identifier. That is a difference in what the two kernels will say about an
anchored child, not by itself a defect.

## Local diagnosis

A local macOS 26.6.2 burn-in reproduced the failure on its second ordinary,
unsandboxed run of `cargo test --locked --test local_protocol`. The audit again
showed no Runyte signal before completion. Unified kernel logging instead
reported that the child hit an unrecoverable exception, and its diagnostic
report identified the termination as `EXC_BREAKPOINT`/`SIGKILL` in the
Foundation namespace with these details:

```text
BUG IN CLIENT OF LIBPLATFORM: os_once_t is corrupt
libsystem_c: crashed on child side of fork pre-exec
```

The stack ran from `_os_once_gate_corruption_abort` through
`_notify_fork_child`, `libSystem_atfork_child`, and `fork` to Rust's
`Command::spawn` and `GitCliProvider::spawn`. Several children in both the
passing and failing attempts produced reports with that same shape. The audit
names the intended Git command while the diagnostic report names Runyte
because the child died before replacing its process image with Git.

The cause was therefore inside Runyte's launch strategy, not memory pressure
or code-signing rejection. `GitCliProvider::spawn` installed a
`Command::pre_exec` hook to call `setsid`. That forces a child-side fork path in
the multithreaded editor. macOS can run libSystem and libnotify at-fork handlers
in that child while another thread has left their one-time initialization state
in progress; libplatform treats the inherited state as corrupt and deliberately
aborts before `exec`.

## Fix

Git needs a private process group so cancellation can terminate Git, hooks,
filters, and network helpers as one owned unit. It does not otherwise depend on
a private session. The launch now uses Rust's safe `CommandExt::process_group(0)`
configuration instead of a child-side `pre_exec(setsid)` hook. The child still
leads a group whose identifier is its PID, preserving all ownership and cleanup
invariants, while macOS can use the standard spawn path without running the
unsafe at-fork sequence.

The Darwin parallel-group test mirrors the production launch configuration and
continues to verify that every leader owns its group and retains the correct
wait status under concurrent completion.

A sequential ordinary-host burn-in ran the complete `local_protocol` test
binary 30 times after the change. All 30 attempts passed. Their 30 process
audits contained 2,939 Git child completions and no completion by signal 9;
every sampled spawn reported the child as its own process-group leader. macOS
also produced no new Runyte diagnostic report during the burn-in.

A separate question stays open regardless of the answer. Discovery currently
latches a signal-terminated child as a Git failure for the workspace. A child
that never ran is not an authoritative statement about the repository, and the
distinction between discovery failure and authoritative absence already exists
in the capability snapshot. Whether an aborted discovery should be retained as
the repository's state is a product decision, not a test-synchronization one,
and it has not been made.
