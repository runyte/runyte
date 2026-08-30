---
title: "Short test runtime roots are duplicated and hardcode `/tmp`"
status: resolved
reported: 2026-08-30
resolved: 2026-08-30
commit: d870473
---

## Resolution

Commit d870473 (`Harden cross-platform integration test boundaries`) added
`TestRuntimeRoot` in `src/test_support.rs` as the single owner of short test
runtime paths. The allocator canonicalizes the advertised temporary base,
measures the platform's complete Unix-socket path budget, and falls back to
canonical `/tmp` only when the advertised base cannot fit Runyte's endpoint.
It creates collision-resistant labelled roots atomically with mode `0700`,
uses an owner marker to guard cleanup, rejects child paths that can escape the
root, and retries an exclusively created name rather than trusting a PID.

Persistent-host fixtures in `src/main.rs`, `src/workspace/catalog.rs`,
`src/workspace/lifecycle.rs`, `src/workspace/transport.rs`,
`tests/diagnostic_log.rs`, `tests/local_protocol.rs`,
`tests/persistent_host.rs`, and `tests/workspace_bulk.rs` now use that policy.
The process-wide local-protocol fixture registers marker-guarded cleanup for
normal process exit because Rust does not drop static values. No fixture
changes a process-global temporary-directory variable.

Coverage is provided by
`test_support::tests::long_advertised_temporary_base_falls_back_with_socket_budget_intact`,
`test_support::tests::advertised_temporary_alias_is_canonicalized`,
`test_support::tests::concurrent_allocations_and_a_stale_pid_name_never_collide`,
`test_support::tests::a_stale_candidate_name_forces_an_exclusive_allocation_retry`,
`test_support::tests::cleanup_refuses_a_directory_whose_owner_marker_changed`,
and `test_support::tests::private_children_cannot_escape_the_owned_runtime_root`
in `src/test_support.rs`.

Known limitation: `SIGKILL` and process abort cannot run either `Drop` or the
normal-exit hook, so a uniquely named stale root can remain. The socket budget
also reserves sixteen bytes for suite-local wrapper components by convention;
a future longer wrapper must update that reserve and its coverage.

## Report

Tests that start local persistent hosts need short Unix-domain socket paths.
Several helpers therefore construct roots directly below `/tmp`, including
code in `src/main.rs`, `src/workspace/catalog.rs`,
`tests/diagnostic_log.rs`, `tests/persistent_host.rs`,
`tests/local_protocol.rs`, and `tests/workspace_bulk.rs`. Related unit tests in
other modules have since added more copies of the same policy.

The copies differ in naming, canonicalization, collision handling, and
permission setup. They bypass `TMPDIR` without one shared statement of why the
platform temporary directory is unsuitable. macOS also advertises some
temporary paths through `/var` while canonicalizing them below `/private/var`,
so a fixture that does not canonicalize its root can compare two spellings of
the same directory.

One test-runtime-root policy should own:

- selection and canonicalization of a short temporary base;
- the measured Unix-socket path budget, including the endpoint suffix Runyte
  adds;
- collision-proof per-test names containing a short diagnostic label;
- owner-only permissions where a runtime directory requires them; and
- cleanup that cannot target another test's directory.

Unit and integration tests may need separate wrappers because integration
tests link a normally compiled library, but both should use the same policy
and naming rules. Environment-derived locations must remain injectable and
tests must not write to the repository, a person's configuration, platform
cache, or real persistent-session state. Process-global environment mutation
is not an acceptable way to isolate parallel tests.

Validation should cover a long advertised temporary directory, the macOS
`/var` versus `/private/var` alias, concurrent allocation, stale names left by
a reused PID, and the final socket path length on supported Unix platforms.
