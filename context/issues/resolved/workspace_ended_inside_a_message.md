---
title: "A persistent session ended with a workspace transport error instead of quitting"
status: resolved
reported: 2026-08-27
resolved: 2026-08-27
commit: fe67314
---

## Resolution

Commit `fe67314` (`Stop truncating workspace transport frames`) fixes two
places where the transport left half a framed message on the socket. The
reader is not at fault: `MessageReader::read` in `src/workspace/transport.rs`
reports an error when the stream ends with bytes still pending, because a
message that ends inside itself is exactly what a truncated write looks like.
Both defects were introduced by `1618ea4` (`Harden local protocol framing`)
the day before the report.

The first is the host's exit path. `run_host_server` in `src/main.rs`
answered a shutdown request with `try_send(HostResponse::ShuttingDown)` and
set `shutting_down`, and there was no await point between leaving the loop and
returning, so the Tokio runtime tore down every connection task on the way
out. A task interrupted mid-write truncated whatever it was sending. The new
`flush_connections` drops the response senders, which closes each channel and
lets the connection task deliver what is already queued — `ShuttingDown`
included — before closing its socket on a frame boundary; waiting for the
resulting `Disconnected` events keeps the runtime alive until that has
happened, under a three-second budget. The endpoint is unpublished before the
flush rather than after, because the listener is still accepting while
established connections drain and a client that discovered the endpoint in
that window would attach to a host with no loop left to answer it.

The second is the framed write itself. `write_message_with_timeout` wrapped
`write_all` in a whole-message `tokio::time::timeout`. Abandoning a write is
safe before its first byte and never after, so firing that deadline part-way
through truncated the frame. The budget now covers a single write and
restarts whenever the peer accepts any byte at all, which still ends a
connection that has genuinely stopped reading; the constant is renamed
`CONNECTION_WRITE_STALL` to say so.

Both defects depend on a write blocking part-way, which is why the report came
from macOS. Its default `net.local.stream.sendspace` is 8 KiB, so an editor
frame never reaches the kernel in one write and a connection is almost always
mid-message. Linux's default unix-socket send buffer is around 212 KiB, so an
ordinary frame goes out in a single write and the window is small enough to
look like it does not exist. The truncation error now names the number of
bytes that were pending, which is what identified the socket buffer as the
boundary involved.

Coverage:

- `tests/local_protocol.rs::a_shutting_down_host_finishes_its_last_message_before_exiting`
  pipelines a mebibyte `ReadBuffer` reply and `Shutdown` without reading
  either, so the reply is certainly in flight when the host leaves its loop.
  It reproduces the reported error on Linux without the fix.
- `src/workspace/transport.rs::a_slow_but_reading_peer_receives_a_whole_message`
  drains a message through a pipe far smaller than it, pausing between reads,
  and asserts both that the transfer outlasts the stall budget and that the
  peer reads the message whole.
- `src/workspace/transport.rs::closing_the_response_channel_delivers_what_is_queued_before_disconnecting`
  covers the contract `flush_connections` depends on: dropping the sender
  writes what is queued and then ends the stream cleanly.

Run `cargo test --test local_protocol` and `cargo test --lib transport` for
those boundaries.

A 2026-08-28 follow-up made the connection task prefer its bounded response
queue over a simultaneously ready socket read. The shutdown flush already
closed the queue and waited for disconnection, but the task's fair outer
selection could accept a wait client's periodic `WaitStatus` poll after the
host had queued `WaitState` and `ShuttingDown`. The host loop had already
ended, so that request could not be answered; under an unlucky schedule the
client observed the socket closing before either queued lifecycle reply was
written. Semantic replies now drain first. The direct interactive regression
`an_interactive_quit_flushes_its_shutdown_response_without_a_control_client`
and the Git wait regression
`git_commit_wait_tui_completes_through_write_quit`, both in
`tests/local_protocol.rs`, cover the two affected paths.

Known limitation: a peer that has genuinely stopped reading still loses the
message being written to it, both when the stall budget expires and when the
shutdown flush budget does. Nothing can be delivered to a peer that is not
accepting bytes, and the connection is destroyed in either case, so the
truncation reaches a stream that has no further use.

## Report

A persistent session on macOS ended unexpectedly. Runyte closed and printed:

```text
Error: workspace transport ended inside a message
```

No other information was available from the session, and the conditions that
produced it were not identified from the report alone. The report raised
whether the project needed a logging facility in order to diagnose failures of
this kind.

Expected behavior is that a persistent session ends the way it was asked to
end — the client detaches or the host reports that it is shutting down — and
that no ordinary lifecycle event surfaces a transport error to the person
using the editor.
