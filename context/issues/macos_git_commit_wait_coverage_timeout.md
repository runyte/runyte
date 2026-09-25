# Git commit wait times out during macOS coverage measurement

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

## Suggested fix method

Reproduce on native macOS with instrumentation, first using the focused target:

```sh
cargo llvm-cov --locked --test local_protocol git_commit_wait_closes_its_buffer_without_detaching_an_existing_tui -- --exact --nocapture
```

Then exercise the full `local_protocol` target under instrumentation and normal
parallelism. A focused pass alone does not rule out interaction with concurrent
tests. Add bounded failure diagnostics that distinguish the Git child, its
editor-wait child, the host's pending wait request, and commit-buffer state.
Inspect whether `:wbc` completed successfully: receipt of a frame alone is not
proof that the command succeeded or the wait response reached its client.
Keep captured diagnostics limited to fixture data and do not publish unrestricted
logs or environment contents.

Fix the boundary established by that evidence. If the fixture advances before
a required semantic event, replace the assumption with an explicit bounded
acknowledgement. If the host or wait client loses completion, correct its
delivery or cleanup ownership and add a deterministic regression. Do not
increase deadlines, serialize the entire suite, or rerun CI until green as a
substitute for diagnosis.

Retain assertions that Git succeeds with the intended commit message and the
existing TUI remains attached. After the fix, run the Rust handoff gates, native
macOS lifecycle stress, and the canonical
`cargo llvm-cov --locked --workspace` measurement. Both Linux and macOS must
retain the 89% floor, and publication requires green CI for the exact release
commit as specified in `context/reference/releasing.md`.
