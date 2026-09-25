---
title: "Node conformance readiness diagnostics and buffered deadline responses"
status: resolved
reported: 2026-09-21
resolved: 2026-09-25
commit: aadaf48
---

## Resolution

Commit `aadaf48` (`Make Node conformance deadline reads message-aware and diagnostic`)
fixed the buffered deadline response and improved timeout evidence. The
publication test previously waited on descriptor readiness even when its
reader had already queued a complete response. The shared `node_reader.py`
reader now consumes queued messages first and uses an absolute deadline, so
fragmented reads cannot renew the three-second ordinary response or
nine-second publication budget. A timeout reports its phase, elapsed time,
child exit state, bounded partial frame and one bounded nonblocking stderr
read without changing the protocol or the timeout assertions.

Coverage is in `docs/plugins/check_node_reader.py`:

- `test_coalesced_deadline_reply_is_returned_without_more_descriptor_readiness`
  verifies an already queued publication reply.
- `test_split_utf8_and_frames_keep_one_absolute_deadline` and
  `test_partial_reads_do_not_restart_the_response_budget` verify fragmented
  replies preserve the deadline.
- `test_timeout_reports_bounded_partial_output_and_stderr`,
  `test_eof_and_invalid_frames_keep_their_distinct_failures`, and
  `test_stderr_snapshot_performs_only_one_bounded_nonblocking_read` verify
  bounded, distinct failure evidence.

All six reader tests and all 14 tests in `docs/plugins/check_node.py` passed.

Known limitation: the cause of the intermittent cold initial-registration
timeout has not been established. The new diagnostics make a recurrence
actionable but do not prevent it.

## Report

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
