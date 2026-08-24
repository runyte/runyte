---
title: "Delayed LSP edits can overwrite newer buffer text"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: b355ec2
---

## Resolution

Commit `b355ec2` (`Reject stale LSP edit responses`) fixed request tracking in `App::lsp_request` and workspace-edit normalization in `lsp::flatten_workspace_edit`.

`TrackedRequest` previously retained only the originating buffer and pending response kind. Format, rename, and resolved code-action responses were therefore converted against whatever text the buffer contained when the server eventually answered. Code-action follow-up requests also used the buffer active when the picker action ran instead of the buffer from which the action had been requested. The request now carries the originating buffer revision, edit-producing responses require that revision to remain current, and code-action resolve and execute follow-ups preserve the captured source even if another buffer becomes active.

`DocumentEdit` previously discarded `VersionedTextDocumentIdentifier.version`. It now retains the optional protocol version. A numeric version is accepted only when an already-open matching LSP document still has that exact version; Runyte does not open a closed file and manufacture a fresh version that could accidentally match. Stale responses are refused with a visible error instead of being clamped and applied to newer text.

Tests covering the behavior live in `src/app.rs`: `delayed_format_and_rename_responses_do_not_edit_a_newer_buffer_revision`, `delayed_code_actions_are_not_offered_for_a_newer_buffer_revision`, `resolved_code_action_keeps_its_source_across_an_active_buffer_switch`, `versioned_workspace_edit_is_rejected_after_the_document_advances`, and `numeric_workspace_version_cannot_be_satisfied_by_opening_a_closed_file`. `resource_operations_are_counted_rather_than_applied` in `src/lsp/mod.rs` verifies that workspace-edit document versions survive normalization.

Known limitation: validation and application of several documents is still sequential rather than atomic; that separate behavior is tracked by `context/issues/resolved/atomic_multi_document_lsp_edits.md`.

## Report

Delayed edit-producing LSP responses could be applied to a newer buffer revision than the one for which they were requested.

`TrackedRequest` in `src/app.rs` retained the buffer and pending request kind, but not the originating buffer revision. Format, rename, and code-action responses were consequently applied unconditionally, and their ranges were converted against the buffer's current text. `flatten_edit` in `src/lsp/mod.rs` also discarded versions carried by `VersionedTextDocumentIdentifier`.

For example, a person could request formatting or rename, continue typing before the server answered, and then have the stale server ranges replace unrelated current text.

Edit-producing requests needed to retain the originating buffer revision and applicable protocol document versions. A response whose preconditions no longer matched needed to be rejected, re-requested, or safely rebased, with a visible explanation rather than silently applied. Coverage needed to delay format, rename, and code-action responses until after another edit and prove that newer text could not be corrupted.
