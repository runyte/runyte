---
title: "Language-server transport tasks survive connection teardown"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 277ee71
---

## Resolution

Commit `277ee71` (`fix(lsp): cancel transport tasks when connections end`)
makes `transport::Connection` retain cancellation handles for its framing tasks
on every platform. Previously Unix dropped the task handles and detached the
stderr drain, allowing silent readers and backpressured writers to retain
streams after the manager retired their connection. Connection destruction now
aborts all owned tasks, including the Unix stderr drain. Explicit shutdown
keeps the existing bounded graceful child-exit interval before destruction;
Windows native process ownership is unchanged.

Final integration exposed a shutdown regression in generic stream connections:
`stop` returned immediately without an owned child, so destruction aborted the
writer before it sent a queued `exit` notification. The follow-up retains the
writer's join handle and drains the closed outgoing queue before cancellation.
Writer draining and child exit share the existing absolute 500 ms deadline;
backpressure cannot extend shutdown indefinitely, and cancellation of the stop
future still runs the connection's task-aborting destructor. Windows native
ownership retains the same abort handles.

`dropping_connection_cancels_idle_reader_and_blocked_writer` and
`stopping_connection_cancels_idle_reader_and_blocked_writer` in
`src/lsp/transport.rs` exercise actual stream closure with idle and blocked
peers. Both failed before the original repair. The follow-up's
`stopping_connection_flushes_queued_messages_before_cancelling_tasks` in the
same file queues complete shutdown/exit frames before the writer can first run,
checks their order and stream closure, and confirms cancellation of the idle
reader. `shutdown_stops_the_manager` in `tests/lsp_client.rs` covers the complete
manager shutdown handshake.

## Report

On Unix, stopping or dropping a language-server connection does not cancel its
framing or stderr tasks. `transport::connect` detaches the reader and writer
tasks, and `transport::spawn` detaches the stderr drain. Only the Windows
connection retains task cancellation handles.

An idle reader remains alive while another process retains the server's output
pipe. A writer blocked on a full pipe also remains alive after the outgoing
queue is dropped. Restarting such a server can accumulate tasks, file
descriptors, queued messages, and stderr state for generations the manager no
longer accepts. The editor's generation checks prevent stale results from
being applied, but do not release those resources.

Connection teardown should cancel every task it owns after its existing
bounded graceful process shutdown. Ordinary `Drop` should also cancel them,
including when the manager exits unexpectedly. Preserve Windows native job
ownership and the normal graceful shutdown interval.

The transport boundary can reproduce the leak without launching a process:
connect a pair of Tokio duplex streams, leave the server side open without
reading or writing, and drop or stop the connection. The server side should
observe its peer closing, and the inbox should lose the connection's senders.
A queued message larger than the duplex capacity additionally exercises the
blocked-writer case.
