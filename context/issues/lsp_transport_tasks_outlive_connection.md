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
