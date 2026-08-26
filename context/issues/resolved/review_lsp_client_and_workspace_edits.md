---
title: "Untrusted language-server traffic could bypass bounds, correlation, and document mutation invariants"
status: resolved
reported: 2026-08-25
resolved: 2026-08-26
commit: 8ed0fe3
---

## Resolution

Commit `8ed0fe3` (`Harden LSP client and workspace edits`) tightened the LSP
manager's trust boundary. JSON-RPC frames are capped at 8 MiB, stderr and event
queues are bounded, requests have per-server and global limits, and server
requests have bounded duplicate detection and exact reply correlation.
Malformed response envelopes, initialization results, position encodings,
sync kinds, requests, and notifications now fail closed. Reserved control
capacity keeps cancellation and mandatory replies available under ordinary
request pressure, including when a cancel overtakes its request.

Server generations now isolate restarts, retirement, writer failure, and late
traffic. Shutdown waits briefly for the response before sending `exit`.
Pre-initialization document traffic is bounded and filtered by negotiated sync
capabilities, and failed opens or changes leave documents absent or explicitly
desynchronized until a full-text retry succeeds.

`apply_document_edits` now treats workspace edits as an atomic plan. It accepts
only absolute local file URIs inside the project, rejects malformed or
non-boundary UTF-8/UTF-16 ranges, duplicate insertions and overlapping edits,
and validates every open-document identity, revision, and version before
committing any buffer. Completion edits use the same exact coordinate and
overlap rules. File create, rename, and delete operations remain unsupported
and are counted rather than executed.

Asynchronous results retain their source buffer, revision, server generation,
document guards, and sending server's position encoding. Diagnostics are
accepted only for the live server-owned document and version. Code-action
commands require the exact advertised command name and run only after every
changed target has been synchronized to the issuing live server generation;
commands are suppressed after queue refusal, failed new-document open, or a
cross-language target. Buffer switches and newer revisions retire transient
or navigation results before they can replace current UI state.

Regression coverage is in these tests:

- `src/lsp/mod.rs`: `mutation_positions_must_be_exact_character_boundaries`,
  `file_uris_accept_only_absolute_local_paths`,
  `malformed_workspace_edits_are_rejected_as_a_whole`,
  `text_document_sync_options_gate_each_notification`,
  `execute_command_requires_the_exact_advertised_name`,
  `resolved_command_only_actions_keep_their_execution_step`, and
  `cancellation_and_edit_replies_use_reserved_control_capacity`;
- `src/lsp/transport.rs`:
  `an_announced_oversized_frame_is_rejected_before_allocation`,
  `a_writer_failure_closes_the_manager_generation`, and
  `newline_free_stderr_is_drained_with_bounded_retention`;
- `src/app/tests/language.rs`: the workspace-edit, completion, diagnostics,
  stale-response, restart, backpressure, and code-action synchronization tests,
  including `code_action_command_is_suppressed_when_an_edit_cannot_be_synchronized`,
  `code_action_command_is_suppressed_when_a_new_target_cannot_be_opened`, and
  `code_action_command_is_suppressed_for_a_target_owned_by_another_server`;
- `tests/lsp_client.rs`: the fake-server matrix for malformed initialization
  and JSON-RPC, negotiated capabilities, bounded queues, cancellation,
  duplicate request IDs, generation retirement, and shutdown, including
  `cancellation_that_overtakes_its_request_drops_the_request`.

Known limitation: language-server resource operations do not create, rename,
or delete files. Runyte reports them as skipped, and refuses a dependent
code-action command, because filesystem mutations remain behind the editor's
confirmed filesystem-plan boundary. Real-server smoke coverage remains opt-in;
the trust-boundary behavior is deterministic in the fake-server matrix.

## Report

Runyte's language-server transport, capability handling, asynchronous results,
and document or workspace mutations accept input from external server
processes. All language-server messages must therefore be treated as untrusted
input.

The primary review boundary was `src/lsp/`,
`src/app/language_workflows.rs`, diagnostics, completion integration, and the
LSP tests. The review covered JSON-RPC bounds and correlation, startup and
shutdown, process failure, cancellation, capability gates, stale responses,
document revisions, UTF-8 and UTF-16 position conversion, malformed and
out-of-range edits, overlapping edits, atomic multi-document application,
project-path containment, rename and file operations, diagnostics lifetime,
and request or notification backpressure.

Confirmed transport defects included permissive JSON-RPC envelope and
initialization parsing; unbounded or overly large frame, stderr, event,
pre-initialization, pending-request, and incoming-request state; missing
duplicate request-ID rejection; reply and cancellation starvation; a race in
which cancellation could overtake request registration; unchecked response
writes; and incomplete cleanup or generation isolation during writer failure,
shutdown, restart, and retirement.

Confirmed synchronization defects included notifications sent despite absent
capabilities, save text sent when not requested, document versions that did not
advance after some local changes, edits accepted after a synchronized target
changed, closed, moved, or reopened, and dependent code-action commands that
could run after their edits failed to reach the issuing server. Late responses
and diagnostic publications did not consistently retain enough buffer,
revision, language, generation, encoding, and document ownership provenance to
reject stale or cross-server state.

Confirmed mutation defects included clamping malformed edit positions instead
of rejecting them; accepting positions inside encoded characters, non-local or
relative file URIs, project-external targets, ambiguous workspace-edit shapes,
annotated edits, duplicate insertions, and overlapping edits; and applying one
document before discovering that a later document was invalid. Completion
auxiliary edits could overlap the primary edit, and their later endpoint could
incorrectly determine the caret.

Expected behavior is that protocol frames, queues, retained request state, and
stderr are explicitly bounded; every response, cancellation, and server request
is correlated exactly; malformed capability or JSON-RPC state fails closed;
and server restarts cannot leak old-generation traffic into current editor
state. Document notifications must follow negotiated capabilities and recover
from backpressure with a full-text resynchronization before later semantic
requests.

Workspace edits must validate every target before changing any buffer, remain
inside the project, use exact negotiated coordinate conversion, and reject
malformed or overlapping edits. Asynchronous results and diagnostics must be
discarded when their source revision, document identity, server generation, or
language ownership is no longer current. A compound code action must not send
its command unless all preceding text edits reached the exact issuing server
generation in order.

The defects are reproducible with bounded fake language servers that emit
malformed, oversized, duplicated, out-of-order, stale, cross-generation, or
capability-inconsistent messages; refuse to read or write at protocol
boundaries; saturate request and notification queues; and return multi-file
edits containing invalid positions, overlapping ranges, non-local URIs,
project-external paths, resource operations, or changed document versions.
Real-server coverage is needed only where behavior cannot be established with
one of the pinned fake-server scenarios.
