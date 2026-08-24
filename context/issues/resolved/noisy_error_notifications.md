---
title: "Mistyped keys and unsupported LSP requests inflate the error count"
status: resolved
reported: 2026-08-18
resolved: 2026-08-18
legacy_commit: 6b12c75
---

## Resolution

Commit `6b12c75` (`Gate optional LSP requests on advertised capabilities;
stop retaining No binding`) removed the two producers the report names and
then checked, rather than assumed, that nothing else needed to change.

**The LSP side.** `finish_initialize` in `src/lsp/mod.rs` deserialized the
server's `InitializeResult` and read exactly two fields off `capabilities`:
`position_encoding` and `text_document_sync`. Everything else the server
advertised — `hover_provider`, `completion_provider`,
`signature_help_provider`, `definition_provider`, and the rest — was dropped
when `initialized` went out of scope, and `struct Server` had nowhere to keep
it even if it had been read. With no capability state anywhere in the
process, every one of `RequestKind`'s fifteen variants was sent to every
server regardless of what that server implemented, and a server that did not
implement one answered `Method not found`, which `apply_lsp_response` in
`src/app.rs` turned into `self.error(reason)` — retained at `ERROR` severity
and counted on the status line. Signature help and completion are asked for
on every `(`, `,`, `.`, and `:` typed (`src/app.rs`'s `after_insert`), so
against a server missing `signatureHelpProvider` or `completionProvider`
this was the only unbounded producer of the two.

A new `lsp::Capabilities` (`src/lsp/mod.rs`) reads the fifteen relevant
`ServerCapabilities` fields, each parsed as its own specification shape
rather than as a single boolean — a plain optional-payload field
(`completion_provider`, `signature_help_provider`,
`execute_command_provider`), a `Simple(bool)`/`Options(_)` union
(`hover_provider`, `type_definition_provider`,
`implementation_provider`, `code_action_provider`, plus
`declaration_provider`'s three-variant `DeclarationCapability`), or a
`OneOf<bool, Options>` (`definition_provider`, `references_provider`,
`document_symbol_provider`, `workspace_symbol_provider`, `rename_provider`,
`document_formatting_provider`). Every field defaults to `false`, so an
omitted capability — the ordinary shape of "not implemented" — reads the
same as an explicit `false`. `code_action_provider`'s own
`resolve_provider` flag is read separately, since `ResolveCodeAction` and
`ExecuteCommand` only ever follow a code action or command the server
itself already returned and so gate on their own advertisement rather than
reusing `code_actions`. `Capabilities` is computed once in
`finish_initialize`, held on `Server`, and forwarded to the editor inside
`LspEvent::Ready`, which now carries it alongside the encoding and sync
mode already there; `App` keeps its copy on `ServerState`, next to the
`Encoding` it already tracked per language.

`lsp_request_from` in `src/app.rs` — the single place any
`LspCommand::Request` is ever sent — now asks `Capabilities::supports(&kind)`
before building one. Denying the request there, before it is ever queued to the
manager, means an unsupported request costs no round trip at all rather
than one that is merely answered locally. The denial goes through a new
`App::mark_unsupported`, structurally `mark_unavailable` minus the
notification push: it sets `status`/`status_error` and bumps
`unavailable_revision` the same way, so a request made through an explicit
command (`gd`, `K`, `Space l r`, ...) still turns into
`CommandOutcome::Unavailable` through the existing
`CommandState::outcome`/`report_completed_action` pipeline — the
`unavailable_revision` check there runs unconditionally, ahead of the
`Asynchronous` hint those commands carry — and is reported on the
interaction line the same way any other unavailable action already is, per
`error_text_in_the_interaction_line.md`. A request made as a typing side
effect (signature help, completion) has no `resolved_binding` and so never
reaches `report_completed_action` at all, matching how an ordinary
character never echoes; it is silent, as the linked
`context/issues/resolved/unsupported_lsp_requests.md` originally asked for. Neither
path retains anything. A `Method not found` that still arrives from a server
that *did* advertise the capability is untouched: it never reaches the new
check, so it still becomes `Response::Failed` and still goes through
`self.error`, retained as before.

**`No binding: X`.** `error_unretained`, structurally `error` minus the
notification push, replaces the single `self.error(...)` call in the
`GrammarNotice::NoBinding` arm of `apply_editor_intent`. `status` and
`status_error` are set exactly as before — the key hints read the grammar
notice directly and are unaffected — but nothing is pushed to
`NotificationCenter`.

**Part 3, confirmed rather than assumed:**

- *Does the notifications buffer refresh while open, or only show what was
  there when it opened?* It refreshes live. `App::push_notification`
  unconditionally calls `refresh_notification_buffers`, which rewrites the
  content of every open `[notifications]` buffer, independent of
  `acknowledge`. A notification that arrives while the buffer is on screen
  appears in it without being closed and reopened; only the *unread count*
  depends on when `acknowledge` last ran, which is the behavior the report
  itself already predicted ("anything arriving after that moment
  legitimately raises the count again"). New test:
  `the_notifications_buffer_refreshes_while_open_and_the_new_entry_counts_as_unread_again`
  in `src/app.rs`.
- *Is the count recomputed correctly from the snapshot in persistent mode
  as it is from live state in standalone mode?* Yes, because there is no
  separate recomputation to diverge:
  `WorkspaceHost::prepare_frame_with_hints` in `src/workspace/host.rs` calls
  `App::snapshot`, the same method standalone rendering calls, fresh on
  every frame it prepares, and that method reads
  `self.notifications.unread_counts()` directly — there is no
  cached or wire-derived count on either side of the boundary, only the one
  `NotificationCenter` computation serialized over the wire. This was
  already exercised end to end by the existing
  `detach_reattach_preserves_notification_history_and_unread_state` in
  `tests/persistent_host.rs`, which retains an error in a detached host,
  reattaches and checks the count survived
  (`unread_errors(&retained) == 1`), then opens `:not` in the same session
  and checks it cleared (`unread_errors(&notifications) == 0`) — both
  already passing before this change and unaffected by it.

Tests, each named with the file it lives in:

- `src/app.rs`: `a_server_that_never_advertised_signature_help_never_gets_the_request`
  and `a_method_not_found_from_an_advertised_capability_is_still_a_retained_error`
  (new) cover the gate itself in both directions.
  `the_notifications_buffer_refreshes_while_open_and_the_new_entry_counts_as_unread_again`
  (new) covers Part 3(a).
  `registry_dispatches_arbitrary_sequences_and_reports_invalid_keys`
  (existing) gained an assertion that a `No binding` sequence leaves the
  notification count untouched.
- `tests/persistent_host.rs`: `detach_reattach_preserves_notification_history_and_unread_state`
  (existing, unmodified) covers Part 3(b) end to end.
- `src/notification.rs`: `history_is_newest_first_bounded_and_acknowledged`
  (existing, unmodified) covers acknowledgement and unread-count semantics
  at the `NotificationCenter` level that Part 3(b) builds on.

Known limitation: the gate is all-or-nothing per request kind. A server that
advertises `signatureHelpProvider` with a `triggerCharacters` list narrower
than Runyte's own hard-coded `(`/`,` (or `completionProvider` narrower than
`.`/`:`) still gets asked on every one of Runyte's trigger characters, not
only the ones the server named; `unsupported_lsp_requests.md` describes this
as a further refinement, not required to remove the unbounded-growth case
this report was about, since the gate already turns every one of those
requests into a zero-round-trip local denial rather than a retained error
regardless of which trigger character produced it.

## Report

The status line shows an unacknowledged error count such as `E2`. Opening
`:not` sometimes clears it and sometimes does not, and the entries that are
there are mostly not worth having been kept:

```
ERROR · 2026-08-18 09:19:01 · Runyte · Action failed · repeated 2 times
No binding: .
────────────────────────────────────────

ERROR · 2026-08-18 09:18:46 · Runyte · Action failed · repeated 4 times
signature help: Method not found
```

Neither of those is an error in any sense that should reach a count on the
status line. Typing faster than the keymap can follow produces the first; a
language server that does not implement an optional request produces the
second.

`signature help: Method not found` has a cause rather than a classification
problem, and is reported separately in `unsupported_lsp_requests.md`: Runyte
never reads what a language server said it supports, so it asks every server
for everything and retains every refusal. That is the largest producer of the
entries seen here, because it is the only one that grows without bound as
typing continues, and gating those requests removes them rather than
reclassifying them.

`No binding: X` should stop being retained at all. It is already shown in the
interaction line and in the key hints at the moment it happens, and it says
nothing that is worth reading later. A burst of mistyping then stops adding to
a count that will not go away.

The count itself may need nothing beyond that. `acknowledge` in
`src/notification.rs:198` marks everything up to the current sequence as read
when the notification buffer opens, so anything arriving after that moment
legitimately raises the count again — including a language-server response
already in flight and the keys pressed on the way to `:not`. Once the two noise
producers are gone this should be confirmed rather than assumed, and two things
in particular are worth checking: whether the notification buffer refreshes
while it is open or only shows what was there when it was opened, and whether
the count is recomputed correctly from the snapshot in persistent mode as it is
from live state in standalone mode.
