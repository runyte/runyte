# Node conformance readiness diagnostics and buffered deadline responses

The Linux plugin-conformance job in
[CI run 35590400592](https://github.com/runyte/runyte/actions/runs/35590400592)
failed the first Node test,
`test_actual_publication_deadline_leaves_host_headroom_and_ignores_late_success`,
inside `opened()` → `handshake()` → `read()` in `docs/plugins/check_node.py`.
The initial registration exceeded the three-second response bound and reported
`Node response timed out`. The other thirteen Node tests passed. This occurred
before a command or publication-deadline timer had been started.

The reader uses unbuffered subprocess pipes, `os.read`, and its own complete-
message queue. Inspection found no lost-readiness explanation for the initial
handshake failure. Cold startup or scheduling delay is plausible but unproven;
the failure currently omits child exit state, partial output and stderr. A
timeout should retain bounded, nonblocking diagnostics identifying the awaited
phase and elapsed time while preserving the existing failure and deadlines.

Separately, the publication-deadline test calls `selector.select(9)` directly
after reading the publication request. If that read also queued the timeout
response, the selector can wait for additional pipe bytes despite a complete
response already being available. Route this deadline wait through the same
message-aware reader, with an explicit deadline. This latent gap does not
explain the observed initial-registration timeout.

Keep the production protocol and timeout assertions unchanged. Validate both
an already-buffered response and a genuine timeout with bounded diagnostics.

The same initial-registration failure recurred in the Linux plugin-conformance
job of [CI run 35647413375](https://github.com/runyte/runyte/actions/runs/35647413375).
Authenticated job logs again place it before publication timing, with the other
thirteen Node tests passing. The initial registration cause remains unproven.

The conformance harness now uses `node_reader.py` for its existing message queue
and an explicit absolute deadline. The publication wait consumes already queued
messages, retaining its nine-second deadline; ordinary replies retain three
seconds. Failure evidence includes phase, elapsed time, child exit state, byte
counts, at most 256 partial-frame bytes and one nonblocking stderr read capped at
1024 bytes. Six deterministic tests in `docs/plugins/check_node_reader.py` pass
on Windows without Node or Unix pipes, covering queued and fragmented replies,
unchanged deadlines, bounded diagnostics and malformed framing. This corrects
the separate buffered-response defect and improves the next timeout report; it
does not establish that cold registration is fixed. The real Node suite remains
part of Unix CI acceptance.
