---
title: "A persistent TUI can exit successfully before reaching a created worktree"
status: resolved
reported: 2026-08-31
resolved: 2026-09-01
commit: 70d64bb
---

## Resolution

Commit `70d64bb` (`Keep persistent TUI connections draining during redraws`)
fixes the unexplained clean exit. `run_attached` read host responses and drew
them on the same task. Ratatui writes a frame synchronously to the outer
terminal, so a backpressured terminal stopped the client from reading its
local socket. Once that exceeded the transport's bounded write-stall budget,
the host correctly released the stalled attachment. The client then treated
the resulting unannounced end of stream exactly like an explicit
`ShuttingDown` response and returned success.

Interactive clients now move response reading onto a dedicated OS thread
after sending the handshake. The nonblocking socket reader waits with `poll`,
so it remains independent even when synchronous drawing blocks a one-worker
Tokio runtime. It feeds the same bounded response-channel contract used by the
host: semantic replies retain FIFO order and their existing fixed capacity,
while visual updates occupy one replaceable slot. Socket delivery can let the
host base terminal damage on a frame the blocked renderer has not consumed,
so the reader applies each delta to its own latest complete wire frame before
entering that slot. The renderer therefore always converges on a
self-contained frame without unbounded buffering. An EOF that was not
preceded by `Detached` or `ShuttingDown` is now an attachment error rather
than a successful user exit.

The first implementation put that reader in a spawned Tokio task. Review
found that `TOKIO_WORKER_THREADS=1`, or a default runtime on a one-CPU
allocation, still let synchronous terminal drawing occupy the only worker and
recreate the same socket backpressure. The dedicated thread closes that gap;
its shutdown clone and joined teardown also keep reader lifetime owned by the
client.

`workspace::transport::tests::buffered_client_drains_the_socket_while_its_consumer_is_blocked`
in `src/workspace/transport.rs` blocks the consuming runtime worker for longer
than the host's write-stall budget on a one-worker runtime while a response
larger than a Unix socket buffer is delivered by another thread, and proves
the independent reader retains the attachment. `killing_the_host_fails_an_attached_persistent_tui` in
`tests/local_protocol.rs` proves an unannounced host loss exits a real PTY TUI
with failure. `creating_a_worktree_starts_and_attaches_its_persistent_session`
in the same file continues to cover the reported worktree-creation path.

Known limitation: a client process that is completely suspended, or whose
dedicated socket-reader thread is not scheduled past the host's bounded
write-stall budget, can still lose its attachment. The host must retain that
bound so a genuinely abandoned client cannot occupy the only interactive slot
indefinitely; such a loss is now reported as failure and the persistent host
retains the workspace for reattachment.

## Report

`creating_a_worktree_starts_and_attaches_its_persistent_session` in
`tests/local_protocol.rs` failed intermittently when whole copies of the suite
ran concurrently on a loaded machine. The attached persistent client exited
with a success status before the newly created worktree's host became
reachable and reported an interactive attachment. The line number below
predates the change that converted the suite's attachment polls and does not
resolve against current `tests/local_protocol.rs`.

```text
thread 'creating_a_worktree_starts_and_attaches_its_persistent_session'
panicked at tests/local_protocol.rs:2573:13:
create-and-attach TUI exited before reaching the new worktree: exit status: 0
```

The clean exit made this notable: the client did not crash, and nothing asked
it to quit. The test drives the worktree view by writing
`\tncreated-from-ui\r<path>\r` to the client's terminal once the host reports
that the `[git worktrees]` buffer contains the project root. That establishes
what the host holds, not what the client has drawn or which state it is ready
to receive keys in, so the original run did not establish whether the client
quit because those keystrokes reached it in a state it was not yet in, or
because it completed a switch the test then failed to observe.

The assertion reports the captured terminal screen and a bounded tail of the
client's PTY output, and two recurrences captured that way had the same shape.
In both, the client's drawing stopped part way through the destination prompt
and was followed immediately by the terminal restore sequence, and the
rendered screen was left empty: the client stopped mid-prompt and then left
the alternate screen deliberately rather than dying inside it. In one capture
it had drawn `worktree destination: /` and was accepting the typed path one
character at a time, reaching column 77 of the 80-column prompt line before it
exited; in the other the drawing stopped at column 80 of the line above the
prompt.

Both recurrences stopping near the right edge of a line raised the length of
the typed destination path as a candidate trigger. That candidate was
untested. The test was run 21 times in isolation with project roots of 103 to
123 characters, produced by pointing `TMPDIR` at directories of increasing
name length, and passed every time, but those runs carried no weight either
way: the failure did not reproduce in isolation at all, so they would have
passed whether or not path length mattered. Varying the path length under the
concurrent load where the exit occurred was not done.

The expected behavior was that a persistent client driven through worktree
creation reached the created worktree's session, and that the test was
deterministic under concurrent execution of the whole suite at its normal
parallelism. Whether the correction belonged in the editor or in the test was
open: a persistent client that exited cleanly while it should still be
attached would be a defect in the editor rather than an artifact of its tests.
The correction could not weaken the lifecycle-stress gate with retries,
ignored failures, reduced parallelism, or serialized tests.

The failure was observed five times across three campaigns totalling 160
whole-suite executions. The campaigns ran against successive versions of the
suite's polling helpers, which changed the deadline around this test and the
diagnostics it printed, but not the exit it reported. Reproduce by building
the suite and running whole copies of it concurrently under saturating load,
which was eight copies at `--test-threads 8` on a twenty-core machine with
thirty busy loops:

```sh
cargo test --locked --test local_protocol --no-run
BIN=target/debug/deps/local_protocol-<hash>   # the path --no-run just printed
loaders=()
for i in $(seq 1 30); do (while :; do :; done) & loaders+=($!); done
for round in $(seq 1 6); do
  copies=()
  for copy in $(seq 1 8); do
    "$BIN" --test-threads 8 > "r${round}-c${copy}.log" 2>&1 &
    copies+=($!)
  done
  for pid in "${copies[@]}"; do wait "$pid" || true; done
done
kill "${loaders[@]}"
```

The round barrier waits on the recorded test copies rather than on every
background job, because a bare `wait` would also wait on the busy loops and
never return.

The test did not reproduce in isolation. It was run 30 times filtered to
itself, as six concurrent copies over five rounds under the same load, and
passed every time. At the rate seen in the whole-suite runs, 30 runs was too
few to distinguish contention between the suite's concurrent tests from
machine load alone, so that observation did not identify which of the two
matters.
