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
