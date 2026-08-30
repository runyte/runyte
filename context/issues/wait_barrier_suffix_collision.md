# Test barrier marker names can collide when the base path has an extension

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
