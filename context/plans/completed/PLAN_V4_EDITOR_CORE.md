# Runyte V4 editor core

Status: completed 2026-07-28

Created: 2026-07-27

## Decision

V4 replaced the original minimal editing layer with the foundations of the
current modal editor. The work established one transactional text model,
normalized multi-range selections, statically linked syntax highlighting,
asynchronous language-server integration, editable directory buffers, and a
single declarative keymap registry.

The editor core follows these rules:

- character offsets are the internal text coordinate;
- every buffer mutation is a transaction with an inverse;
- selections are normalized ranges with one primary range;
- key execution, help, hints, and command metadata share one registry;
- tree-sitter details remain inside `src/syntax/`;
- language-server JSON-RPC remains inside `src/lsp/` and never blocks input or
  rendering; and
- filesystem changes proposed through a directory buffer require a typed plan
  and explicit confirmation before application.

## Implemented foundation

### Text and selections

`src/text.rs` owns rope-backed text, character offsets, transactions, and
transaction inversion. `src/buffer.rs` owns file-backed and generated buffers,
saved state, and transactional undo groups. `src/selection.rs` owns normalized
multi-range selections. Editing commands map changes across the complete
selection and commit one logical action as one undo step.

Unicode scalar boundaries are preserved internally. Terminal cell width and
row/column projection are presentation concerns derived at the appropriate
boundary rather than stored as competing text coordinates.

### Syntax and language services

Tree-sitter grammars are statically linked. `src/syntax/` is the only layer
aware of `tree-house`; callers receive scope values and character offsets.
Incremental parsing runs away from the input path and applies a completed tree
only to the revision it was built for.

`src/lsp/` owns language-server transport and protocol types. One Tokio task
per client handles JSON-RPC while the editor uses a non-blocking handle and
drains events between input frames. Buffer revisions guard edits returned by a
server so stale responses cannot modify newer text.

### Directory buffers

An explorer is an editable directory projection with hidden stable entry
identities. Editing its text prepares an `FsPlan`; no filesystem operation is
performed until the plan has been reviewed and confirmed. Plan construction,
conflict checks, cycle-safe application, and trash behavior live below the
editor integration in `src/directory_buffer.rs` and `src/fs_plan.rs`.

### Commands and presentation boundaries

`src/command.rs` defines command identities and metadata. `src/keymap.rs` is
the single source of truth for dispatch, help, and key discovery. The headless
facade and owned snapshots expose editor semantics for tests and bundled local
frontends without becoming a public RPC or extension API.

## Current records

Current behavior and limitations are documented in:

- `README.md` and `docs/user-guide.md` for users;
- `context/reference/helix-keymap-v1.md` for binding compatibility and
  deliberate deviations;
- `context/reference/ui-vocabulary.md` for presentation terminology; and
- the architecture map in `AGENTS.md` for current module ownership.

This plan records why the foundation has its present shape. The current source
and reference documents are authoritative where later work refined it.
