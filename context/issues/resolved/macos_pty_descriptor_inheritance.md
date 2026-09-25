---
title: "macOS PTY descriptors may be inherited during allocation"
status: resolved
reported: 2026-09-21
resolved: 2026-09-25
commit: 566b4fc
---

## Resolution

Commit `566b4fc` (`Allocate macOS PTY endpoints with atomic close-on-exec`)
replaces the macOS allocation-to-flag-update window in
`src/terminal/pty.rs::open_pair`. A native overlap test against the previous
allocator observed both endpoints in an unrelated executed child, with
`F_GETFD` equal to zero for both. Setting flags after that execution could not
repair the child's inherited copies.

macOS now creates the master with `posix_openpt(O_CLOEXEC)`, grants and unlocks
its peer, obtains its name through `TIOCPTYGNAME` into caller-owned storage,
and opens the slave with `O_CLOEXEC`. Both opens also retain `O_RDWR` and
`O_NOCTTY`. Every successful allocation immediately enters `OwnedFd` ownership;
fallible setup and injected failures release the endpoints. The slave exists
before sizing, and sizing precedes the intended child's `setsid`, controlling
terminal acquisition and standard-descriptor duplication. No helper or local
spawn mutex is needed, and there is no inheritable allocation fallback on
macOS or Linux.

The native compiled probe now inventories `/dev/fd` on macOS and distinguishes
endpoints by character-device identity. Allocation cannot proceed past either
checkpoint until the executed probe acknowledges its observation through an
owned socket. The Linux `/proc/self/fd` and `TIOCGPTN` probe remains intact.
The updated native test observes neither endpoint after exec.

Regression coverage in `src/terminal/tests/pty_descriptors.rs`:
`allocation_endpoints_do_not_survive_unrelated_exec`,
`allocation_failure_closes_every_owned_endpoint`,
`endpoint_identity_distinguishes_simultaneous_ptys`, and
`allocated_slave_has_initial_size_and_observes_master_resize`.
`a_child_sees_the_size_the_pty_was_opened_with`, `input_reaches_the_child`,
`a_child_writes_to_the_pty_and_then_exits`,
`every_post_spawn_setup_failure_terminates_and_reaps_the_child`, and
`running_child_teardown_still_signals_and_reaps_its_private_group` in
`src/terminal/pty.rs` cover the intended child lifecycle.
`pending_terminal_failed_and_cancelled_preparations_release_the_gate_and_lease`
and
`pending_terminal_cancel_releases_reader_accounting_while_an_external_slave_stays_open`
in `src/terminal/pending/tests.rs` cover unpublished cancellation. These tests
passed on native aarch64 macOS.

Known limitation: close-on-exec does not prevent temporary inheritance between
fork and exec. Other Unix targets retain the native `openpty` fallback without
this atomic guarantee. Native Linux validation remains a CI responsibility;
the native measurements in this resolution are macOS results.

## Report

The macOS branch of `src/terminal/pty.rs::open_pair` uses native `libc::openpty`
to allocate the master and slave, then sets `FD_CLOEXEC` on each endpoint with
separate `fcntl` calls. An unrelated process executing between allocation and
those updates can retain an endpoint. Updating the parent's flags afterward
does not change the child's copy. A retained endpoint can extend PTY lifetime
or affect shutdown and EOF behavior.

The remaining platform gap is documented in
[`terminal-compatibility-v1.md`, Unix PTY descriptor ownership](../../reference/terminal-compatibility-v1.md#unix-pty-descriptor-ownership).
The Linux fix in commit `5e30ffb` uses atomic close-on-exec allocation; it does
not change the macOS path. Apple's published
[`openpty` implementation](https://github.com/apple-oss-distributions/Libc/blob/main/util/pty.c)
opens both endpoints without `O_CLOEXEC`. Native macOS flags and an actual
concurrent exec overlap still need verification. The Linux reproduction is
not evidence of a macOS runtime failure.

An unrelated executed program must not retain either PTY endpoint. The intended
terminal child must still receive stdin, stdout, stderr, its controlling
terminal and process group, and the requested initial size. Close-on-exec does
not prevent temporary inheritance between fork and exec; a child that forks
and deliberately never executes is outside this guarantee.

A deterministic native regression should hold allocation before the flag
updates, execute an unrelated compiled fixture through `std::process::Command`,
and identify retained endpoints by native device identity rather than descriptor
numbers alone. The fixture needs an explicitly owned control channel, execution
and observation acknowledgments, bounded waits, and cleanup on every failure
path. Linux's `/proc/self/fd` and `TIOCGPTN` probe in
`src/terminal/tests/pty_descriptors.rs` needs a native macOS equivalent. Fixed
sleeps, probabilistic stress, and cross-compilation do not establish the
inheritance guarantee.

The allocation strategy remains undecided pending native verification. A
separate flag update merely narrows the race. A local mutex cannot serialize
unrelated standard-library spawns. A helper process is not predetermined.
Preserve the native setup ordering: both endpoints exist and initial sizing is
applied to the slave before the child calls `setsid` and acquires its controlling
terminal. The earlier manual sequence failed with `ENOTTY`, as recorded in
[`terminal_panes.md`](terminal_panes.md); the FFI pointer requirements
are recorded in [`macos_openpty_build.md`](macos_openpty_build.md).

Validation must cover deterministic overlapping exec, allocation-failure
cleanup, terminal input/output, initial size, resize, process-group teardown,
and pending-terminal cancellation on native macOS. Keep the existing Linux
regression and the canonical Linux/macOS coverage floor intact. Fixtures must
use compiled or checked-in executables, temporary storage and configuration,
and deliberate control-descriptor inheritance, without changing process-global
environment variables in concurrent tests.
