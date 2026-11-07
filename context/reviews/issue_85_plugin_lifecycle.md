# Plugin lifecycle marker backpressure

Investigation for [#85](https://github.com/runyte/runyte/issues/85), based on
`3a5c9a6d`, on 2026-11-07. This record retains the controlled macOS reproduction
and validation evidence for review; transfer it to the fixing PR when publishing.

## Environment

- macOS 26.6.2 (25G83), `aarch64-apple-darwin`.
- Mac14,2, eight logical CPUs, 24 GiB RAM.
- Rust 1.97.1 (`8bab26f4f`), LLVM 22.1.6.
- Native local execution with Unix sockets and child processes permitted.
- Separate worktree and build directory. Another worktree was concurrently
  building and testing the editor and desktop crates. Observed load averages
  ranged from about 2 before builds to about 23 during shared activity.

The sandbox-only first attempt was denied permission to bind its endpoint;
that failure is not reproduction evidence. Subsequent runs permitted the local
socket/process operations required by the fixtures.

## Cause and controlled reproduction

The test's `send_input` reads one response after sending F12. That response can
be an asynchronous frame published before the input was processed. The old
marker helper then polls only the filesystem for five seconds, leaving the
interactive socket unread.

`serve_connection` in `src/workspace/transport_shared.rs` finishes a queued
response write before reading another client request. If an unread frame fills
the socket, the F12 request remains behind that write. The worker is registered
and alive, but never receives `command.invoke`. More time polling the marker
does not fix the ownership of the socket reads.

The added regression requests a larger frame and a bounded burst of 32
resynchronizations before F12, spacing them across render turns to prevent all
redraws from coalescing. This deliberately creates the unread-frame ordering;
it does not depend on a slow plugin or extend an acceptance deadline.

With reading disabled in the marker helper, this regression failed on macOS:

```text
stable plugin worker did not reach accepted finite work after 5.000077875s;
drained 0 frames; host exit=Some(Ok(None)); worker running=Some(true)
application-worker.trace: hello received; register sent
registered
application-worker.stderr:
host.stderr:
```

The bounded host input trace recorded frame publications through frame 4 and
no F12 dispatch. There was no `command.invoke`, `view.create`, `pane.show`, or
`job.create` exchange after registration. Both processes were still alive.
An earlier version of the same queued-redraw experiment also failed with its
trace ending at registration.

With concurrent reading restored, the queued-frame test drained 19 frames and
reached accepted finite work in 23.787 ms; the original case drained two frames
and reached it in 12.844 ms in that focused run. Both completed all original
job/view persistence, repeated attachment, detached completion, single-worker,
shutdown/reaping, and unchanged-file assertions.

One hundred repeated focused runs without the injected redraw burst passed
before the fix, under mixed shared-machine load. The initial parallel stress
sequence also passed its first four complete iterations; its fifth iteration
was stopped for the controlled experiment and final validation. Some later
baseline iterations included preliminary forms of the added regression. These
passes do not reproduce or explain the historical CI scheduling precisely.
The controlled failure establishes a concrete fixture synchronization defect
with the same stopped stage; the original CI run had insufficient diagnostics
to prove its exact ordering retrospectively.

## Change

The marker wait now drains visual responses while checking the marker, under
the unchanged five-second total deadline. It does not resend F12 or retry the
plugin invocation. Registration and detached completion still wait without an
interactive client. Test-only diagnostics record bounded protocol-stage labels,
response errors, worker liveness/natural exit, host exit, and isolated host and
plugin stderr. Output removes fixture paths and derived workspace identifiers.
There is no production transport, plugin, or timeout change.

## Validation

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed, including a final check
  after correcting the enlarged fixture geometry's status/message rows.
- `cargo test`: passed on native macOS with ordinary test parallelism. This
  overlapped the other worktree's desktop checks.
- Ten focused runs of both lifecycle cases under two additional owned
  `/usr/bin/yes` CPU workers: all 20 cases passed. The workers were terminated
  and reaped after the check. Accepted finite work took 12.758–65.144 ms.
- Ten complete iterations, each running `local_protocol`, `diagnostic_log`,
  `persistent_host`, and `workspace_bulk` with normal parallelism: all 40 test
  binary invocations passed (1,040 passing test cases). No failed gate was
  retried. Both lifecycle cases ran in every iteration; accepted finite work
  took 17.938–67.377 ms across their 20 observations. Later iterations ran
  after the other worktree's heavy checks had finished.

`cargo-llvm-cov` is not installed on this machine, so no new canonical coverage
measurement was produced. Production source and the coverage floor are
unchanged. Linux and Windows were not run locally; this Unix integration-test
change does not change desktop or production behavior.
