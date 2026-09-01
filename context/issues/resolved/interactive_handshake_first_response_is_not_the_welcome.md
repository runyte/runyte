---
title: "The first response on a new interactive connection was not always the welcome"
status: resolved
reported: 2026-08-31
resolved: 2026-09-01
commit: aec9644
---

## Resolution

Commit `aec9644` (`Order connection handshakes before visual updates`) fixed
the ordering in `serve_connection` in `src/workspace/transport.rs`. The host
already sent `Welcome` before publishing the initial frame, but
`ResponseReceiver::recv` tried to preserve that order by merging the semantic
queue and replaceable visual slot with a biased `tokio::select!`. Bias only
orders branches that are ready when each branch is polled. The receiver could
poll an empty semantic queue, have the host enqueue `Welcome` and a frame, and
then observe the now-ready visual branch during that same poll. The frame was
therefore returned and written before the semantic branch was polled again.

`ResponseReceiver::recv_handshake` now waits on the semantic queue alone, and
`serve_connection` writes that response before entering the loop that
multiplexes semantic replies, final messages, visuals, and client requests.
This makes the first-response rule a connection-transport invariant rather
than a scheduler-dependent consequence of select bias. It retains the
replaceable visual slot and all later asynchronous frame behavior; no retry,
test serialization, or protocol-shape change was needed.

Tests covering the behavior are:

- `interactive_handshake_precedes_a_visual_that_woke_the_writer_first` in
  `src/workspace/transport.rs`, which wakes the writer with a frame before
  supplying `Welcome` and requires the welcome to reach the client first;
- `revision_protocol_is_stale_safe_undoable_and_bounded` and
  `incompatible_worktree_host_returns_the_tui_to_its_source` in
  `tests/local_protocol.rs`, the two integration paths from the report.

## Report

Two tests in `tests/local_protocol.rs` fail intermittently when whole copies
of the suite run concurrently on a loaded machine, and both failures are
consistent with one symptom: the first response delivered on a newly
established interactive connection was not the `Welcome`.

One of the two observes it directly. The editor's own client reports the
response it received and exits, and the test that was driving that client
then fails at a later wait because the client is gone. Line numbers in these
excerpts predate the change that converted the suite's attachment polls and
do not resolve against current `tests/local_protocol.rs`.

```text
thread 'incompatible_worktree_host_returns_the_tui_to_its_source'
panicked at tests/local_protocol.rs:884:13:
buffer "[git worktrees]" did not contain
"/tmp/runyte-local-protocol-387582-1788163093622498611-13/linked" after 30s;
last contents: "<buffer was not opened>"; terminal output: [...]
Error: unexpected workspace handshake response: Frame { frame: HostFrame {
id: FrameId(10), active_buffer: BufferId(1), ... } }
```

That message is produced by `run_attached` in `src/main.rs`, which reads the
first response on a new interactive connection and requires it to be
`Welcome`. The captured PTY shows the client entering and then leaving the
alternate screen before printing the error, so the terminal guard had run and
the handshake failed immediately afterwards.

The second test infers the same interleaving from the other side. It connects
one interactive client, discards one response as the welcome, and requires
the next to be the initial frame:

```text
thread 'revision_protocol_is_stale_safe_undoable_and_bounded'
panicked at tests/local_protocol.rs:1466:21:
expected initial frame, got Welcome { protocol: 43, pid: 291984,
features: [Snapshots, Input, Buffers, Wait], host_version: "0.1.6" }
```

For the second response to be the `Welcome`, the first must have been
something else. What it was is not recorded. It was not a `Refused`: the
host drops the response sender at the end of that branch in `src/main.rs`,
so a refused connection is closed and never receives a `Welcome` after it.
The other failure shows a `Frame` in that position.

Two places decide the order, and on the reading below it should hold. The
host's accept path in `src/main.rs` sends `Welcome` into the connection's
semantic response channel and only then calls `publish_attached_frame`. The
per-connection carrier those go through, `response_channel` in
`src/workspace/transport.rs`, is not one queue: semantic responses keep FIFO
order in an `mpsc`, final messages have a one-slot queue of their own, and
complete frames and terminal damage share a replaceable `watch` slot.
`ResponseReceiver::recv` merges the three with a biased `select!` that polls
the semantic queue first, so a welcome already sitting in that queue should
reach the wire ahead of a frame published afterwards. Why it did not was
unidentified in the original report.

The expected behavior is that a client connecting interactively receives its
`Welcome` first. Where the correction belongs was initially open. The
positional read in `revision_protocol_is_stale_safe_undoable_and_bounded` was
worth hardening either way, because the file already documents that frames
and terminal deltas are uncorrelated with a request and provides
`receive_semantic_response` for reading past them, but hardening that test
would only stop it observing the ordering rather than establish that the
ordering holds. The correction must not weaken the lifecycle-stress gate with
retries, ignored failures, reduced parallelism, or serialized tests.

The two failures were observed across three campaigns totalling 160
whole-suite executions: `revision_protocol_is_stale_safe_undoable_and_bounded`
twice, and `incompatible_worktree_host_returns_the_tui_to_its_source` once.
The campaigns ran against successive versions of the suite's polling helpers,
which changed the deadlines around these tests but not what either failure
reports. Reproduce by building the suite and running whole copies of it
concurrently under saturating load, which was eight copies at
`--test-threads 8` on a twenty-core machine with thirty busy loops:

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

Neither test reproduces in isolation. Each was run 30 times filtered to
itself, as six concurrent copies over five rounds under the same load, and
passed every time. At the rate seen in the whole-suite runs, 30 runs is too
few to distinguish contention between the suite's concurrent tests from
machine load alone, so that observation does not identify which of the two
matters.
