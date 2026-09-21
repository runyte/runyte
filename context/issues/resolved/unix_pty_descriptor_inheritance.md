---
title: "Unix PTY descriptors may be inherited by unrelated child processes"
status: resolved
reported: 2026-09-21
resolved: 2026-09-21
commit: 5e30ffb
---

## Resolution

Commit `5e30ffb` (`Allocate Linux PTY endpoints atomically close-on-exec`)
fixes the Linux allocation race in `terminal::pty::open_pair`. `openpty` created
inheritable endpoints, and the following `fcntl(F_SETFD, FD_CLOEXEC)` calls could
not revoke copies already inherited by an unrelated executed child.

On Linux x86-64, kernel `7.2.5-200.fc44.x86_64`, glibc 2.43, a private checkpoint
held the vulnerable implementation after `openpty` returned but before either
flag update. The regression launched the compiled unit-test executable through
`std::process::Command`; the executed fixture reported both exact endpoints:

```text
allocation_endpoints_do_not_survive_unrelated_exec
assertion `left == right` failed: PTY endpoints survived exec; F_GETFD=[0, 0]
  left: [1, 1]
 right: [0, 0]
```

The executed fixture enumerates `/proc/self/fd`, comparing device/inode/rdev
identity plus `TIOCGPTN` for masters. `fstat` alone can identify the common
`/dev/ptmx` device rather than one terminal session. A socket pair created
close-on-exec is intentionally mapped onto the fixture's stdin; framed execution,
observation and release acknowledgments keep allocation held until observation
completes. Read/write deadlines and an owned child guard bound failure paths.
The test runs a compiled executable, creates temporary configuration storage,
and changes no process-global environment. An initial sandbox run failed with
`EPERM` while configuring the socket deadline; the native reproduction and
validation run outside that sandbox. That sandbox failure is not the bug.

Linux now creates the master with `posix_openpt` and the slave with
`TIOCGPTPEER`, both using `O_RDWR | O_NOCTTY | O_CLOEXEC`. `OwnedFd` owns each
endpoint before fallible preparation. Allocation failures therefore close every
endpoint obtained so far. Peer allocation requires Linux 4.13 or later, below
the supported Ubuntu 22.04 release-build environment, and fails rather than
falling back to inheritable allocation when the ioctl is unavailable or denied.
Initial sizing remains on the open slave; the intended child's `setsid`,
controlling-terminal acquisition, standard-descriptor duplication and
process-group cleanup are unchanged. The regression now holds both the master-only
and paired allocation boundaries while an unrelated fixture executes; neither
endpoint survives. A deliberately vulnerable negative control was used during
local validation and removed afterward to avoid leaking descriptors into other
parallel tests. Independent review covered reproduction and implementation.

Local Linux formatting and denied-warning all-target Clippy passed, and the
full ordinary suite passed 3,851 tests with 36 ignored. Test and coverage builds
used one Cargo job and two test threads to respect
the machine's available memory; the final Clippy check allowed two jobs.
The canonical local workspace coverage command with `--summary-only
--fail-under-lines 89` also passed: 126,013 of 137,092 lines, **91.92%**.

Native [CI run 35575280176](https://github.com/runyte/runyte/actions/runs/35575280176)
for implementation commit `5e30ffb` passed ordinary Linux and macOS tests,
repeated lifecycle suites on both platforms, and canonical instrumented workspace
coverage: 91.92% total lines on Linux and 91.84% on macOS, above the unchanged
89% floor. These checks preserve existing macOS behavior; they do not reproduce
or close its separate inheritance window. Independent validation review found
no remaining actionable code findings.

Coverage at the behavior boundary is provided by:

- `allocation_endpoints_do_not_survive_unrelated_exec`,
  `allocation_failure_closes_every_owned_endpoint`, and
  `endpoint_identity_distinguishes_simultaneous_ptys` in
  `src/terminal/tests/pty_descriptors.rs`.
- `a_child_writes_to_the_pty_and_then_exits`,
  `a_child_sees_the_size_the_pty_was_opened_with`, `input_reaches_the_child`,
  `every_post_spawn_setup_failure_terminates_and_reaps_the_child`,
  `completed_child_teardown_never_signals_a_reusable_process_group`, and
  `running_child_teardown_still_signals_and_reaps_its_private_group` in
  `src/terminal/pty.rs`.
- `pending_terminal_failed_and_cancelled_preparations_release_the_gate_and_lease`,
  `pending_terminal_cleanup_kills_descendants_after_the_unreaped_leader_exits`,
  and
  `pending_terminal_cancel_releases_reader_accounting_while_an_external_slave_stays_open`
  in `src/terminal/pending/tests.rs`.
- `typing_in_insert_mode_reaches_the_child`,
  `terminal_insert_swap_keeps_the_live_child_and_resizes_at_its_new_geometry`,
  and `closing_a_terminal_ends_its_child_and_forgets_it` in `tests/terminal.rs`.
- `terminal_pid_output_and_input_survive_detach_disconnect_and_reattach` in
  `tests/persistent_host.rs`.

Known limitation: macOS retains native `openpty` followed by flag updates;
its allocation-to-update inheritance window remains open and is tracked in
[`macos_pty_descriptor_inheritance.md`](../macos_pty_descriptor_inheritance.md).
The current platform boundary is documented in
[`terminal-compatibility-v1.md`](../../reference/terminal-compatibility-v1.md#unix-pty-descriptor-ownership).
No native macOS overlap reproduction is claimed. Close-on-exec does not prevent
temporary inheritance between fork and exec, including a child that deliberately
never executes. Windows mixed-launch handle inheritance is outside this fix.

## Report

### Evidence and expected behavior

Source inspection on 2026-09-21 at `dev` commit `5e8a436` found a potential
descriptor-inheritance race in `src/terminal/pty.rs::open_pair`. The function
calls `libc::openpty`, takes ownership of the returned master and slave, then
sets `FD_CLOEXEC` on each endpoint with separate `fcntl` calls. The source does
not establish atomic close-on-exec creation. The effective flags returned by
the supported platform implementations and an actual overlapping launch need
native verification. At report time, no Linux runtime failure had been
reproduced.

If either endpoint is initially inheritable, an unrelated process launched
between creation and the later `fcntl` can retain that endpoint after exec.
Changing the parent's descriptor flags afterward does not change the child's
copy. A retained endpoint can extend PTY lifetime or affect terminal shutdown
and EOF behavior. Such symptoms are possible consequences, not established
diagnoses of a previously observed Linux test failure.

An unrelated executed program must not retain either endpoint of a Runyte
terminal session. The intended terminal child must still receive its standard
input, output and error, controlling terminal, process group and initial size.
Ordinary Git subprocess pipes on Linux already use Rust's atomic
`pipe2(O_CLOEXEC)` path; the PTY allocation boundary needs separate inspection.
Close-on-exec does not prevent temporary descriptor inheritance between fork
and exec, and this issue does not promise isolation from a child that forks
and deliberately never executes another program.

### Native reproduction and regression boundary

The requested regression boundary was a deterministic Linux reproduction before
selecting a fix:

1. Inspecting both endpoints requires a private test synchronization point
   holding the vulnerable implementation after `openpty` returns and before
   either close-on-exec update, with no production public API exposure.
2. While allocation is held, an unrelated compiled fixture launched through
   `std::process::Command` must actually exec. Its retained endpoints must be
   checked by endpoint identity rather than only descriptor numbers, which
   can be reused by the fixture's runtime.
   On Linux, PTY device identity may require more than `fstat` alone.
3. Observations must return through an explicitly owned control channel before
   releasing the allocation barrier, with bounded waits, explicit acknowledgments
   and cleanup on every failure path. Fixed sleeps and probabilistic stress
   loops are not the primary reproduction.
4. The boundary requires a demonstrated vulnerable failure and a retained
   regression forcing the equivalent concurrent-launch opportunity during
   corrected allocation, proving neither endpoint survives exec. It also
   requires allocation-failure cleanup and preserved normal terminal input,
   output, resize and shutdown behavior.

Fixtures use a compiled test executable or an existing checked-in executable;
they must never execute files written by the test. Storage and configuration
belong to temporary fixture directories. Every editor or host subprocess sets
fixture-owned `XDG_CONFIG_HOME`, and tests must not change process-global
environment variables in a concurrent test binary. Fixture control descriptors
must themselves have deliberate inheritance and close-on-exec ownership.

### Implementation scope and constraints

The implementation constraint was to close the demonstrated allocation race
with the smallest reliable native boundary. An atomic close-on-exec allocation
path is preferred where supported. Setting the flag earlier in a separate syscall
merely narrows the window. A new local mutex is insufficient unless every
potential competing launcher participates, including standard-library spawns.
At report time, syscall or allocation strategy selection was undecided pending
reproduction and platform verification; a helper process was not predetermined.

Linux is the native implementation environment. The current PTY function is
shared with macOS, so macOS behavior must be preserved and checked in CI. A
Linux-only fix must explicitly identify any remaining macOS gap rather than
claiming all Unix platforms are fixed. No parser, keymap, terminal rendering,
persistent-session protocol, or Windows process-launch redesign is required by
this issue. Process-group identity and bounded cleanup guarantees must remain
intact.

The existing `openpty` boundary replaced a manual allocation sequence that
failed on macOS with `ENOTTY`; `context/issues/resolved/terminal_panes.md`
records the diagnosis. Initial sizing must remain on the slave, with correct
controlling-terminal acquisition after `setsid`; the old sequence must not
return in a Linux allocation path. The related FFI portability
record is `context/issues/resolved/macos_openpty_build.md`.

Repository guidance, integrated-terminal behavior in `docs/user-guide.md`,
`context/README.md`, and `context/reference/terminal-compatibility-v1.md`
constrain the implementation. Relevant implementation and
coverage live in `src/terminal/pty.rs`, `src/terminal/pending/`,
`src/process_group.rs`, `tests/terminal.rs`, and `tests/persistent_host.rs`.
`src/git/cli.rs` provides a comparison of subprocess pipe ownership;
the existence of a PTY gap is not evidence that Git's Linux pipes have it too.

### Validation and delivery

Validation requires recorded native evidence and independent review of each
work package, with actionable findings incorporated before the next package.

Required validation includes the focused overlap regression and affected
PTY/lifecycle tests on Linux, followed by the repository handoff checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The canonical `cargo llvm-cov --locked --workspace` line-coverage floor in
`context/reference/test-coverage.md` must hold on every affected first-class
target. macOS validation requires remote CI, with exact failing assertions and
test names retained if a platform remains unresolved. A passing stress run or
cross-compilation alone does not establish the inheritance guarantee.

The report scoped implementation to `dev`, independently of Windows Git work
on `feat/windows-support`. That branch had already merged into `dev` before
this fix began. Windows has a separate potential mixed-launch handle
inheritance problem; neither platform's failure is proof of the other's cause.
The report requested focused Unix implementation and regression commits suitable
for cherry-picking into `feat/windows-support`, with exact hashes and validation
results, without merging unfinished Windows work into `dev`. Issue resolution
requires the separate follow-up commit specified by `AGENTS.md`, preserving the
diagnosis and any platform limitation.
