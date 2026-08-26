---
title: "Malformed local peers could retain host connection state and exceed framing bounds"
status: resolved
reported: 2026-08-25
resolved: 2026-08-26
commit: 1618ea4
---

## Resolution

Commit `1618ea4` (`Harden local protocol framing`) made established connection
cleanup unconditional. `serve_connection` now retains the result of its
established loop, emits `ServerEvent::Disconnected` after clean EOF, malformed
JSON, truncated input, read failure, or write failure, and only then returns
the result. Protocol validation and role errors cross the same FIFO server-event
boundary as valid requests, so an error cannot overtake the semantic response
to an earlier unnumbered request.

The protocol now validates the complete hello request before publishing a
connection, bounds frame geometry against both rectangle containment and a
32,768-cell worst-case terminal serialization budget, limits client
notifications, and rejects unknown fields in request-owned structures. An
exhaustive role check keeps interactive input and semantic commands on the
interactive connection while retaining the control connection's bounded
buffer, wait, health, preview, rename, and lifecycle operations.

`LocalServer` limits concurrent connection tasks and retains a permit for each
task until every exit path completes. Framed writes have a deadline so a peer
that stops reading cannot retain an attachment indefinitely. `LocalClient`
removes and shuts down its writer after any failed send, preventing a later
request from being appended to a partially written JSON frame. The existing
cancel-safe `MessageReader` remains the single owner of partial input, and the
8 MiB frame limit still applies before deserialization.

Regression coverage is in these tests:

- `src/protocol/mod.rs`:
  `protocol_version_and_request_bounds_are_explicit`,
  `request_decoding_rejects_unknown_fields`, and
  `maximum_terminal_geometry_fits_the_wire_frame_budget`;
- `src/workspace/transport.rs`:
  `invalid_handshake_fields_are_refused_before_connection`,
  `malformed_established_messages_always_disconnect_host_state`,
  `requests_outside_the_connection_role_receive_an_error`,
  `incomplete_handshakes_cannot_grow_connection_tasks_without_bound`,
  `a_peer_that_stops_reading_cannot_hold_a_write_forever`,
  `a_timed_out_client_writer_is_poisoned`, and
  `an_established_stalled_peer_disconnects_and_releases_its_slot`;
- `tests/local_protocol.rs`: the complete local-protocol integration suite,
  including revision-safe buffer changes, `--wait`, attachment, host failure,
  and persistent-session switching.

A 2026-08-26 test-portability follow-up made that integration boundary
observable rather than timing-dependent. Real-TUI cases continuously drain
their PTYs and observe rendered output before sending input, asynchronous
generated views are awaited through buffer reads, and read-only wait responses
are checked at the synchronous response-to-frame classification boundary,
where an older queued visual frame cannot be misattributed to the poll.

Known limitation: Serde ignores extra object members on fieldless request
variants such as `Health`. The explicit `type` still selects the complete
operation, those members cannot alter it, and unknown message kinds remain
errors, so the private bundled-client contract does not add a second parsing
layer solely to reject semantically inert members.

## Report

Runyte's private versioned protocol is used only by bundled local clients. Its
primary implementation boundary is `src/protocol/` and
`src/workspace/transport.rs`, with end-to-end coverage in
`tests/local_protocol.rs`.

The hardening review covered frame-size and collection bounds, partial reads
and writes, malformed and truncated payloads, unknown fields and message kinds,
version negotiation, request identity, timeout behavior, slow and disconnected
peers, serialization failures, input-event validation, memory growth, and the
distinction between interactive and control connections. It was proactive;
changes were limited to defects confirmed at those boundaries.

An established connection that encountered malformed JSON, a truncated
message, or a socket write failure returned from `serve_connection` before it
published `ServerEvent::Disconnected`. The persistent host could consequently
retain that connection as its active interactive TUI or as a control client
after the transport task had ended.

Handshake fields were deserialized but were not passed through the request's
normal validation. Frame geometry was accepted without an area or containment
limit, so a local peer could request a snapshot large enough to allocate and
serialize far beyond the transport's 8 MiB message limit. Client-originated
notification text had no field-specific bound. Unknown request-owned fields
were accepted by the default derived deserializer.

Connection tasks had no total concurrency bound. Incomplete handshakes had a
deadline, but a burst could retain one task and partial-message buffer per
accepted socket until that deadline. Established writes had no deadline, so a
peer that stopped reading could retain a task and attachment. Adding a write
deadline also required the client writer to become unusable after failure,
because cancelling `write_all` can leave a partial JSON frame on the stream.

Request permissions were enforced incompletely above the transport. Several
requests sent on the wrong role received no response, while control
connections could reach the semantic-command path only to be rejected later.
Directly generated transport errors also used a different response path from
host-generated semantic replies and could overtake an earlier pipelined reply;
the protocol has no request identifier with which a client could disambiguate
that ordering.

Expected behavior is that frame sizes, collections, request fields, geometry,
and retained connection resources are explicitly bounded; partial reads and
writes cannot desynchronize a reusable stream; malformed, truncated, or
unknown message kinds cannot panic the host or leave connection state retained;
unknown fields in field-carrying request values are rejected; and
interactive and control connections accept only their intended operations.
Version and workspace identity mismatches must be refused before a connection
is published. Semantic responses and protocol errors must retain request order
without turning the bundled-client protocol into a public compatibility
contract.

The defects can be reproduced with private Unix-socket peers that complete a
valid handshake and then send malformed or truncated JSON, stop reading a
large response, advertise oversized or out-of-screen geometry, pipeline a
valid control request before a role-invalid input request, or occupy the
listener with incomplete handshakes. Boundary tests use private temporary
runtime directories and do not write into repository or user runtime state.
