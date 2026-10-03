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
