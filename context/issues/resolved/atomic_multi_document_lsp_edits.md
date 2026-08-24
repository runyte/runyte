---
title: "Multi-document LSP edits could apply only partially"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: c39cac0
---

## Resolution

Commit c39cac0 (`Apply LSP workspace edits atomically`) replaced
`apply_document_edits`' sequential open-and-mutate loop with a prepare/commit
boundary. The old loop could edit one live buffer before a later target failed
validation, after which Runyte rejected the request despite retaining the
earlier mutation.

The preparation phase now confines and identifies every path, reconciles
document versions, groups aliases and duplicate targets, opens or clones every
buffer, builds each transaction, rejects grouped overlaps, applies transactions
only to staged buffers, and parses new syntax state. No live editor buffer is
changed until all targets succeed. The commit phase swaps the staged buffers
into existing slots and reconciles syntax, LSP notifications, guards, and pane
selections; new buffers are appended with their already-prepared text and
syntax. It contains no recoverable fallible operation.

Canonical longest-existing-prefix identities are used only for grouping and
matching. The retained target path remains the buffer's path, so symlink aliases
and versioned LSP documents do not create duplicate buffers. Duplicate edit
ranges that `Transaction::new` would otherwise discard are rejected as an
atomic request failure, while ordered same-position insertions remain
supported.

Coverage lives in `src/app.rs`. Run `cargo test workspace_edit --lib`; the
focused tests include
`a_later_invalid_document_rejects_a_workspace_edit_atomically`,
`duplicate_workspace_edit_documents_form_one_transaction`,
`overlapping_duplicate_workspace_edits_are_rejected_atomically`,
`a_versioned_workspace_edit_reuses_an_open_symlink_alias`, and
`nonexistent_workspace_edit_aliases_have_one_identity`.

## Report

Multi-document LSP workspace edits were applied incrementally rather than
atomically.

`apply_document_edits` immediately opened and changed each document in
sequence. A later document could then fail because its path was missing,
outside the project, unreadable or binary, or read-only. Runyte reported the
overall workspace edit as rejected, but changes already applied to earlier
buffers remained.

A refactor or code action could therefore leave a partial result while both
the user and language server were told it did not apply.

Every document and range needed to be validated first, duplicate paths grouped,
and all transactions constructed before any buffer was mutated. Applying the
planned transactions needed either to complete as one logical operation or
roll back an unforeseen later failure. Coverage needed to place an invalid
second document after a valid first document and verify that neither buffer
changed when the request was rejected.

Relevant code was `src/app.rs` in `apply_document_edits` and workspace-edit
response handling.
