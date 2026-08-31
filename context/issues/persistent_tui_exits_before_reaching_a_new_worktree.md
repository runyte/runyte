# A persistent TUI exits successfully before reaching a created worktree

`creating_a_worktree_starts_and_attaches_its_persistent_session` in
`tests/local_protocol.rs` fails intermittently when whole copies of the suite
run concurrently on a loaded machine. The attached persistent client exits
with a success status before the newly created worktree's host becomes
reachable and reports an interactive attachment. The line number below
predates the change that converted the suite's attachment polls and does not
resolve against current `tests/local_protocol.rs`.

```text
thread 'creating_a_worktree_starts_and_attaches_its_persistent_session'
panicked at tests/local_protocol.rs:2573:13:
create-and-attach TUI exited before reaching the new worktree: exit status: 0
```

The clean exit is what makes this notable: the client did not crash, and
nothing asked it to quit. The test drives the worktree view by writing
`\tncreated-from-ui\r<path>\r` to the client's terminal once the host reports
that the `[git worktrees]` buffer contains the project root. That establishes
what the host holds, not what the client has drawn or which state it is ready
to receive keys in, so the run does not establish whether the client quit
because those keystrokes reached it in a state it was not yet in, or because
it completed a switch the test then failed to observe.

The assertion reports the captured terminal screen and a bounded tail of the
client's PTY output, and two recurrences captured that way have the same
shape. In both, the client's drawing stops part way through the destination
prompt and is followed immediately by the terminal restore sequence, and the
rendered screen is left empty: the client stopped mid-prompt and then left
the alternate screen deliberately rather than dying inside it. In one capture
it had drawn `worktree destination: /` and was accepting the typed path one
character at a time, reaching column 77 of the 80-column prompt line before
it exited; in the other the drawing stopped at column 80 of the line above
the prompt.

Both recurrences stopping near the right edge of a line raises the length of
the typed destination path as a candidate trigger. That candidate is
untested. The test was run 21 times in isolation with project roots of 103 to
123 characters, produced by pointing `TMPDIR` at directories of increasing
name length, and passed every time, but those runs carry no weight either
way: the failure does not reproduce in isolation at all, so they would have
passed whether or not path length matters. Varying the path length under the
concurrent load where the exit does occur has not been done.

The expected behavior is that a persistent client driven through worktree
creation reaches the created worktree's session, and that the test is
deterministic under concurrent execution of the whole suite at its normal
parallelism. Whether the correction belongs in the editor or in the test is
open: a persistent client that exits cleanly while it should still be
attached would be a defect in the editor rather than an artifact of its
tests. The correction must not weaken the lifecycle-stress gate with retries,
ignored failures, reduced parallelism, or serialized tests.

The failure was observed five times across three campaigns totalling 160
whole-suite executions. The campaigns ran against successive versions of the
suite's polling helpers, which changed the deadline around this test and the
diagnostics it prints, but not the exit it reports. Reproduce by building the
suite and running whole copies of it concurrently under saturating load,
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

The test does not reproduce in isolation. It was run 30 times filtered to
itself, as six concurrent copies over five rounds under the same load, and
passed every time. At the rate seen in the whole-suite runs, 30 runs is too
few to distinguish contention between the suite's concurrent tests from
machine load alone, so that observation does not identify which of the two
matters.
