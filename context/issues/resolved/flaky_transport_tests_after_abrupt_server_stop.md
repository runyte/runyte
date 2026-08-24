---
title: "Transport tests fail intermittently after an abrupt server stop"
status: resolved
reported: 2026-08-21
resolved: 2026-08-21
legacy_commit: cea56cd
---

## Resolution

Commit `cea56cd` (`Shut down workspace listener before abrupt stop`) fixed the
accept-loop shutdown in `LocalServer::stop_abruptly`. The helper aborted the
Tokio task and awaited its destruction, but closing the listener alone could
leave a queued Unix-socket connection observable briefly. A registry probe or
replacement bind could therefore still connect after the helper returned.

`LocalServer::bind` now gives its background accept loop an explicit one-shot
shutdown signal. The shutdown branch is biased ahead of another accept, calls
`shutdown(SHUT_RDWR)` while the task still owns the listener descriptor, and
then exits so normal task destruction deregisters and closes the descriptor.
`stop_abruptly` sends that signal and awaits the completed task while still
leaving the endpoint files behind, preserving the crashed-host state these
tests need. Ordinary `LocalServer` drop also signals the loop, aborts it as a
fallback, and continues to remove the published endpoint synchronously. The
listener remains in the background task so handshakes can still complete
before the host polls its event receiver.

The behavior is covered in `src/workspace/transport.rs` by
`dead_registry_entries_are_removed_while_listing`,
`live_recorded_process_prevents_socket_unlink_and_transient_errors_are_not_stale`,
and `endpoint_metadata_is_atomic_private_and_stale_socket_recovers`.
`mismatched_handshake_is_actionably_refused` also covers background handshake
acceptance. The complete library suite passed eight consecutive runs, followed
by `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
`cargo test`.

## Report

Two unit tests in `src/workspace/transport.rs` failed intermittently under
`cargo test`, roughly one run in six:

```
workspace::transport::tests::dead_registry_entries_are_removed_while_listing
    transport.rs:2456: assertion failed:
    registered_hosts_in(&[runtime.join("runyte/hosts")]).unwrap().is_empty()

workspace::transport::tests::live_recorded_process_prevents_socket_unlink_and_transient_errors_are_not_stale
    transport.rs:2618: a workspace host is already listening at
    /tmp/runyte-transport-live-process-<pid>-<nanos>/.runyte/host/workspace.sock
```

They failed together or singly depending on the run, and passed when either
was run alone. Each used its own temporary root and, in the first test's case,
its own runtime directory named from the process ID and a nanosecond timestamp,
so the two were not competing for one path.

The failures were not caused by a recent change. They were measured on the
merged work that abbreviated workspace IDs in the session listing, and on
`779f1a4`, which preceded it:

- `779f1a4`: 3 failures in 24 runs of `cargo test --lib`
- merged `main`: 4 failures in 17 runs

The samples were too small to establish whether the rates differed, but the
base failed independently, and the workspace-ID change did not touch sockets
or the registry.

Both tests called `LocalServer::stop_abruptly` and then asserted something
that depended on the endpoint no longer being connectable. The first expected
a registration whose recorded PID was deliberately set to the live test
process to be reaped anyway, because socket liveness rather than PID liveness
decides. The second expected a rebind to be refused specifically because the
recorded process was still alive; the observed failure was that it was refused
for a different reason, because something was still listening.

Both failures were consistent with the listening socket outliving
`stop_abruptly`, which aborted the server task and awaited the aborted handle.
It was initially unconfirmed whether that await was sufficient to guarantee
that the listener had stopped accepting and its socket had closed, and whether
the same state could briefly be visible during an ordinary abrupt host
shutdown.
