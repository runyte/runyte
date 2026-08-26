---
title: "Persistent workspace metadata and lifecycle controls lacked complete bounds"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: ab6ffbb
---

## Resolution

Commit ab6ffbb (`Harden persistent workspace state`) confirmed and corrected
four related hardening gaps. `WorkspaceService::stop` depended on the host
registry even when a live current-protocol endpoint was published for a
directory, so a missing registry row made that host impossible to stop. Stop
resolution now falls back to the directory endpoint, retains the published
protocol, and distinguishes current-protocol control requests from explicitly
forced incompatible-process termination. Only protocol requests use the short
control deadline; incompatible termination keeps its longer bounded exit and
cleanup window. Registry-visible incompatible hosts are listed without an
impossible handshake, refuse non-forced shutdown, and can be force-stopped
without abandoning endpoint cleanup.

`connect_control`, `rename_host`, and shutdown request/response exchanges could
wait indefinitely after connecting to an unresponsive local peer. Each short
lifecycle exchange now has a bounded deadline while interactive attachment
connections remain long-lived.

Workspace recents previously read and deserialized the complete file before
enforcing their entry limit. Recents now cap input at 8 MiB and 256 entries and
validate absolute bounded project paths, session names, and workspace numbers
on both read and write. Endpoint, registry, and stored-name metadata reads are
also bounded and validate PID, workspace identity, absolute path, socket-path,
and name fields before acceptance. Registry discovery streams directory
entries, caps JSON rows before collecting them, and sorts the bounded set for
deterministic processing.

Coverage lives in `src/workspace/catalog.rs` in
`targeted_directory_operations_reach_a_live_host_absent_from_the_registry`,
`a_registered_incompatible_host_lists_without_a_handshake_and_requires_force_to_stop`,
and `recents_reject_oversized_files_and_semantically_unbounded_entries`;
`src/workspace/lifecycle.rs` in `control_handshake_has_a_deadline` and
`lifecycle_request_response_has_a_deadline`; and
`src/workspace/transport.rs` in
`registry_entry_count_is_bounded_while_the_directory_is_streamed`,
`endpoint_metadata_is_bounded_and_validated_before_acceptance`, and
`stored_session_name_input_is_byte_bounded`.

## Report

Persistent-session host ownership and retained workspace state required a
focused hardening review. The scope included host discovery and launch races,
endpoint and lock ownership, stale registrations, catalog corruption,
canonical workspace identity, attachment exclusivity, control connections,
detach and reattach, workspace switching, protected-state checks, idle
retirement, `--wait` ownership, failure recovery, forced shutdown, cleanup,
and isolation between workspaces.

Persistent sessions do not promise survival across host or machine failure.
Regression tests use isolated temporary state roots and do not write runtime
state into the repository or user paths.
