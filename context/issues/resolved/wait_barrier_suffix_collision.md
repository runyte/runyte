---
title: "Test barrier marker names can collide when the base path has an extension"
status: resolved
reported: 2026-08-30
resolved: 2026-08-30
commit: d870473
---

## Resolution

Commit d870473 (`Harden cross-platform integration test boundaries`) added
`marker_path` and `wait_status_barrier_paths` in `src/test_support.rs`.
`marker_path` appends its suffix to the complete `OsString` instead of calling
`Path::with_extension`, so `/tmp/wait.first` and `/tmp/wait.second` now retain
distinct `.ready` and `.release` markers. `wait_at_test_status_barrier` in
`src/main.rs` and its local-protocol producer both use the shared derivation.

The audit also corrected the release marker in `src/external_open.rs` to append
`.release`, matching the checked-in stand-in program's protocol. The existing
one-shot acknowledgement, deadline, environment boundary, and temporary-root
cleanup remain unchanged.

Coverage is provided by
`test_support::tests::wait_barrier_markers_preserve_the_complete_base_name` in
`src/test_support.rs` and
`durable_completion_wins_a_race_with_launcher_loss` in
`tests/local_protocol.rs`.

Known limitation: the implementation preserves non-UTF-8 paths structurally
through `OsString`, but there is no dedicated non-UTF-8 marker regression.

## Report

The wait-status test barrier derives its acknowledgement files with
`Path::with_extension("ready")` and `Path::with_extension("release")` in
`wait_at_test_status_barrier` in `src/main.rs`. The corresponding integration
test in `tests/local_protocol.rs` derives the same paths independently.

`with_extension` replaces an existing extension rather than appending a
suffix. A base such as `/tmp/wait.first` therefore produces
`/tmp/wait.ready`, not `/tmp/wait.first.ready`. Two barriers whose base paths
have the same stem but different extensions can address the same marker
files. A stale or concurrently written marker can then release the wrong
client and turn a synchronization regression into a passing test.

The marker names should preserve the complete base path and append distinct
`.ready` and `.release` suffixes. Production and test code should use one
shared derivation rather than duplicating the naming rule. Other test-only
release markers, including the one in `src/external_open.rs`, should be
audited for the same replacement behavior.

The fix must retain the current one-shot acknowledgement protocol, bounded
deadline, and test-only environment boundary. Marker cleanup must remain
scoped to the owning temporary directory and must not remove a path that could
belong to another test.

Reproduction:

1. Derive marker paths for `/tmp/wait.first` and `/tmp/wait.second` with the
   current implementation.
2. Observe that both produce `/tmp/wait.ready` and `/tmp/wait.release`.
3. Run two users of those paths concurrently and observe that either one can
   satisfy the other's barrier.
