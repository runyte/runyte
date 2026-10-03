---
title: "File monitor can retain its worker after shutdown"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 9671138
---

## Resolution

Commit `9671138` (`fix(files): retain monitor shutdown across queue overflow`)
adds a shared atomic stop flag to `FileMonitorHandle` and its worker.
Previously `drop` depended on inserting a stop message into a bounded queue,
while the worker's own watcher retained a sender that prevented disconnection.
Dropping the handle now publishes termination independently of queue capacity;
the message remains a nonblocking wakeup for an idle worker. The periodic
reconciliation schedule and observation behavior are unchanged.

`dropping_monitor_stops_worker_when_command_queue_is_full` in
`src/file_monitor.rs` fills the command channel, retains the callback sender
and event receiver, and verifies bounded worker completion. All seven tests in
that module pass.

## Report

# File monitor can retain its worker after shutdown

`FileMonitorHandle::drop` sends `WorkerMessage::Stop` with `try_send` and
discards a full-queue error. The native watcher owns another sender to that
same channel inside the worker, so dropping the editor's handle does not
disconnect the receiver. Once queued messages have drained, a worker with no
registered buffers can continue reconciling every two seconds indefinitely;
it has no observation to send that would reveal a closed output channel.

Dropping the monitor must reliably request termination without blocking the
editor, including when its bounded command queue is full. Worker lifetime must
not depend on another filesystem event or on delivering an observation.

A deterministic reproduction fills a bounded command channel before starting
the worker, retains a second sender to model the watcher's callback, and drops
the monitor handle. The queued stop request cannot be inserted. With an empty
registration list, the original worker remains alive even after the event
receiver is dropped. Regression coverage should verify worker completion with
the callback sender and output receiver retained, rather than relying on their
disconnection to end the loop.
