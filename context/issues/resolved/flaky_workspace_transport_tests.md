---
title: "Workspace transport tests fail intermittently during server teardown"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: fae8f08
---

## Resolution

Commit `fae8f08` (`Make workspace server teardown synchronous`) fixed the
failure. `LocalServer::drop` was only aborting the Tokio accept task, and task
abort schedules cancellation without synchronously dropping the listener.
The server value could therefore be gone while its Unix socket still accepted
probes and its registry row still described a live endpoint.

`LocalServer` now retains the `LocalEndpoint` it published and runs the
existing identity-locked, metadata-matched endpoint cleanup during normal
drop. This makes the socket and registry disappear before drop returns while
preserving the guard that prevents an old server from deleting a replacement
server's registration. A test-only abrupt-stop path aborts and joins the
listener without cleanup, so crash-recovery tests still exercise genuinely
stale endpoint state rather than graceful shutdown.

Coverage is in `src/workspace/transport.rs`:

- `endpoint_metadata_is_atomic_private_and_stale_socket_recovers` verifies
  replacement after an abrupt listener stop.
- `old_cleanup_cannot_remove_a_replacement_hosts_registration` verifies that
  normal drop permits immediate replacement and old cleanup cannot remove it.
- `dead_registry_entries_are_removed_while_listing` verifies registry reaping
  after an abrupt stop even when the recorded PID is live.
- `live_recorded_process_prevents_socket_unlink_and_transient_errors_are_not_stale`
  preserves the unsafe-stale-recovery refusal.

The complete `workspace::transport` unit-test module passed 30 consecutive
runs, and `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
`cargo test` passed.

## Report

The workspace transport tests failed intermittently. `cargo test` could pass
on one run and fail on the next without any intervening change, so a single
clean run was not reliable.

Two tests in `src/workspace/transport.rs` failed this way, sometimes both in
the same run.

`old_cleanup_cannot_remove_a_replacement_hosts_registration` panicked at
`src/workspace/transport.rs:2259`, where the second endpoint bound:

```text
called `Result::unwrap()` on an `Err` value: a workspace host for
/tmp/runyte-transport-cleanup-race-3174012-1787177347485472434 is already
running at /tmp/ryt-cr-3174012-975350872/first/workspace.sock
```

The host it refused to displace was the test's own first server, which the
line above had dropped. The runtime directory was named from the process ID
and the clock, so two tests were not colliding on one path.

`dead_registry_entries_are_removed_while_listing` panicked at
`src/workspace/transport.rs:2308`:

```text
assertion failed: registered_hosts_in(&[runtime.join("runyte/hosts")]).unwrap().is_empty()
```

`endpoint_metadata_is_atomic_private_and_stale_socket_recovers` also failed
once. Its message was not captured, and the failure did not recur in thirty
further runs.

When `cargo test --lib workspace::transport` was run in a loop, observed
failure frequencies were three failures in twelve runs, four in fifteen, one
in twenty, and three in twenty-five. Running either failing test alone passed
every time, including twenty-five consecutive runs of the first test.

Both failing tests had the same shape: each dropped a server, awaited a single
`tokio::task::yield_now()`, and then asserted that the socket had been released
or the registration reaped. It was initially undecided whether one yield was
insufficient or whether teardown genuinely remained unfinished when the next
bind or listing observed it. In the latter case, the behavior also affected a
running editor and did not belong solely in the tests.

Neither the module nor its tests had been touched by the editor work in
progress around it; `src/workspace/transport.rs` last changed in `699e817`.
