---
title: "Git commit wait times out during macOS coverage measurement"
status: resolved
reported: 2026-09-25
resolved: 2026-09-25
commit: 9f86723
---

## Resolution

Commit `9f86723` (`Drain Git wait fixture output through commit completion`)
corrects the lifecycle of
`git_commit_wait_closes_its_buffer_without_detaching_an_existing_tui` in
`tests/local_protocol.rs`. The fixture left the Git PTY unread, stopped
consuming the simulated TUI's frames while waiting for Git, and treated receipt
of a frame as evidence that `:wbc` had completed. The shadowed `Command` builder
also retained its slave `Stdio` handles after spawning Git, preventing output
capture from reaching EOF even after all children exited.

Native macOS instrumentation established a deterministic fixture failure by
using a checked-in stand-in as a post-commit hook that emits 128 KiB. With the
previous unread-output behavior, Git hit the existing ten-second timeout while
the host reported zero pending waits and a closed commit buffer. Git's hook
was still running; the simulated TUI had meanwhile been disconnected because
it stopped reading frames. Starting a PTY drain after the timeout let Git exit
successfully. These observations identify fixture backpressure independently
of editor-wait completion.

The fixture now starts a terminal drain immediately, releases the command
builder's slave handles, and owns Git with `ChildGuard` for failure cleanup.
An ordered buffer-list response and a disk-content assertion establish actual
save/close completion. Health requests continue consuming interactive frames
while verifying that the existing TUI remains attached and no wait is pending.
Git still has the original ten-second exit deadline. Timeout diagnostics retain
only the last attachment/wait counts, a bounded inventory of the fixture's
Git/editor descendants, and a bounded terminal-output tail. The test also
requires the hook's completion marker, a successful Git exit, the exact intended
commit subject, and the retained attachment. No production wait lifecycle or
key binding changed.

Regression coverage is
`git_commit_wait_closes_its_buffer_without_detaching_an_existing_tui` in
`tests/local_protocol.rs`; the verbose hook makes removal of the PTY drain fail
deterministically. Native instrumented focused and full-target investigation
included ten full-target passes before the amplified output regression. After
the fix, three additional native full-target stress passes each passed all 44
ordinary tests. The final ordinary and canonical instrumented workspace suites
each passed 3,933 tests with 38 ignored. `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` passed. Canonical native macOS
line coverage is 91.89% (128,260 of 139,585 lines), above the unchanged 89%
floor; see the [coverage register](../../reference/test-coverage.md).

Known limitation: the historical CI timeout's exact trigger was not reproduced
with the original output volume. The diagnosed and corrected boundary is the
fixture's demonstrated backpressure and retained descriptor ownership; this
is not evidence of a production lost-wait defect or causation by the Paramiko
pin. Linux coverage and any release publication gate still require their own
CI results on the exact candidate commit.

## Report

The macOS coverage job for `dev` commit `b39bc86` failed on 2026-09-25 in
`git_commit_wait_closes_its_buffer_without_detaching_an_existing_tui` from
`tests/local_protocol.rs`. The evidence is retained in
[CI run 36105567072, job 107977198811](https://github.com/runyte/runyte/actions/runs/36105567072/job/107977198811).
The job used Rust 1.97.1 on `aarch64-apple-darwin`.

The fixture starts a Git commit using Runyte as its waiting editor while a TUI
is already attached. It inserts `host-owned commit message`, sends `:wbc` and
receives a frame. It then waits for the Git child to exit before checking that
the existing interactive attachment remains active. That child wait failed:

```text
thread 'git_commit_wait_closes_its_buffer_without_detaching_an_existing_tui' panicked at tests/local_protocol.rs:672:6:
child process timed out: Elapsed(())
```

The backtrace identifies the call at `tests/local_protocol.rs:2334` in the
reviewed revision. `wait_child` polls `Child::try_wait` every 25 ms under a
ten-second deadline. This integration target finished with 43 passed, one
failed and one ignored test. The canonical instrumented workspace run therefore
failed before coverage reports and the enforced 89% line-coverage check could
run. This is a test failure during measurement, not evidence that measured
coverage fell below the floor.

The full Linux suite passed outside the review sandbox, and the previous `dev`
commit, `d36c3ab`, had green CI. Neither result establishes the cause of this
macOS failure or clears the candidate's failed release gate. The failing commit
only changes the Paramiko dependency pin; causation by that change has not been
established. Scheduling delay, fixture behavior and a production lifecycle
defect remain possibilities rather than diagnoses.

Expected behavior is that `:wbc` saves and closes the commit-message buffer,
completes the corresponding editor wait, and allows Git to finish without
detaching the existing TUI or disturbing unrelated buffers. Completion must
remain bounded under the canonical macOS coverage run.

### Validation constraints

The report proposed native macOS reproduction with instrumentation, first using
the focused target:

```sh
cargo llvm-cov --locked --test local_protocol git_commit_wait_closes_its_buffer_without_detaching_an_existing_tui -- --exact --nocapture
```

Validation also requires the full `local_protocol` target under instrumentation
and normal parallelism. A focused pass alone does not rule out interaction with concurrent
tests. Bounded failure diagnostics must distinguish the Git child, its
editor-wait child, the host's pending wait request, and commit-buffer state.
Successful `:wbc` completion must be established: receipt of a frame alone is not
proof that the command succeeded or the wait response reached its client.
Captured diagnostics must remain limited to fixture data, excluding unrestricted
logs or environment contents.

The fix must address the boundary established by evidence. Advancement before a
required semantic event calls for an explicit bounded acknowledgement. Lost host
or wait-client completion calls for a correction to delivery or cleanup ownership
and a deterministic regression. Increased deadlines, suite-wide serialization,
and rerunning CI until green are not substitutes for diagnosis.

Assertions must retain Git success with the intended commit message and the
existing TUI attachment. Validation requires the Rust handoff gates, native
macOS lifecycle stress, and the canonical
`cargo llvm-cov --locked --workspace` measurement. Both Linux and macOS must
retain the 89% floor, and publication requires green CI for the exact release
commit as specified in `context/reference/releasing.md`.
