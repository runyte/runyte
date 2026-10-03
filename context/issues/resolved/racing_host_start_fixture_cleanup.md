---
title: "Racing host startup fixture deletes storage before child processes exit"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: d30e21c
---

## Resolution

`d30e21c` (`Wait for both racing host processes before fixture cleanup`)
corrects teardown in
`racing_starts_for_one_workspace_both_reach_the_winning_host`. Endpoint readiness
was mistaken for completion of the losing child's startup, and the shutdown
acknowledgement was mistaken for completion of the winning child's exit.

The fixture now starts each child through a symlink to the checked-in stand-in.
Its behavior data records the shell PID before exec replaces it with the bundled
editor, preserving that PID. Both startup callers still run concurrently through
`start_detached_host`. The fixture keeps the winner serving until the losing
child exits, preventing a late contender from publishing after shutdown. After
`--session-stop`, it waits for both recorded processes to disappear before
removing project or runtime storage. Existing reaper threads retain child wait
ownership. Production startup, shutdown semantics, and deadlines are unchanged.

Validation: `racing_starts_for_one_workspace_both_reach_the_winning_host` in
`tests/persistent_host.rs` passes, as do ten full lifecycle burn-in passes of
`tests/local_protocol.rs`, `tests/diagnostic_log.rs`, `tests/persistent_host.rs`,
and `tests/workspace_bulk.rs` with normal parallelism and Unix socket access.
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`
pass; the ordinary suite passes 4,553 tests with 54 ignored. Canonical Linux
`cargo llvm-cov --locked --workspace` passes at 91.97% line coverage, above the
unchanged 89% floor. Native macOS validation remains CI-owned.

Known limitation: the original CI failure was not reproduced locally, and its
log did not identify the directory entry or process that raced with deletion.
The correction establishes process completion before deleting fixture storage.

## Report

The Ubuntu lifecycle stress job in [CI run
37116524125](https://github.com/runyte/runyte/actions/runs/37116524125/job/111184306937)
failed on burn-in attempt 9 of 10 at commit `5f8d138`.
`racing_starts_for_one_workspace_both_reach_the_winning_host` reached
`fs::remove_dir_all(root).unwrap()` at `tests/persistent_host.rs:2047` after
both startup calls and `--session-stop` succeeded. Directory removal returned
`Os { code: 39, kind: DirectoryNotEmpty, message: "Directory not empty" }`.
The log does not identify the entry created during deletion or its writer.

The fixture starts two detached hosts concurrently for one workspace. Each
`start_detached_host` call returns when the endpoint answers, even if its own
child has not yet finished startup. The losing child is therefore not
necessarily gone when both calls return. The stop command also returns on
`ShuttingDown`, before the winning process has finished cleanup and exited.
Deleting the project immediately after that acknowledgement races with those
processes. A sufficiently delayed contender can also reach bind after the
winner has unpublished its endpoint.

Keep the winning host serving until the losing child exits, then stop it and
wait for both processes to exit before deleting project or runtime storage.
Preserve concurrent startup, normal suite parallelism, and existing deadlines;
fixed sleeps and deletion retries do not establish process completion.

The failing invocation is `cargo test --locked --test persistent_host`, repeated
by the lifecycle stress job among the local protocol, diagnostic log, and
workspace bulk suites. Ten local full persistent-host passes with Unix socket
access did not reproduce the original failure before the correction.
