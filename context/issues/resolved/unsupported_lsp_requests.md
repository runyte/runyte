---
title: "Every language-server request was sent to every server regardless of what it advertised"
status: resolved
reported: 2026-08-18
resolved: 2026-08-19
legacy_commit: a388d4c
---

## Resolution

Commit a388d4c (`Ask on the trigger characters a language server advertised`)
closed this, finishing what 6b12c75 (`Gate optional LSP requests on advertised
capabilities; stop retaining No binding`) began the day before. The report has
two halves — which requests are sent at all, and which keystrokes send them —
and they landed separately.

`finish_initialize` in `src/lsp/mod.rs` deserialized the server's
`InitializeResult`, read `position_encoding`, `text_document_sync`, and
`server_info.name` out of it, and dropped the rest when `initialized` went out
of scope. `struct Server` had no field to hold what the server advertised, so
no capability state existed in the process and `lsp_request_from` in
`src/app.rs` had nothing to consult. 6b12c75 added `lsp::Capabilities`, which
reads each advertised field as the specification shapes it — a plain
`Option`, a `Simple`/`Options` union, or a `OneOf<bool, Options>`, each
through its own small reader rather than one boolean coercion — carries it on
`Server`, forwards it through `LspEvent::Ready`, and stores it per language in
`App`'s `ServerState`. `lsp_request_from` now calls `Capabilities::supports`
before sending anything, and a request the server never advertised is turned
away locally by `App::mark_unsupported`, which sets the interaction line to
"the {language} language server does not support {label}" and bumps
`unavailable_revision` without retaining a notification. The gap belongs to
the server rather than to the editor, so it is reported as unavailable rather
than as an error; a `Method not found` from a server that *did* advertise the
capability still reaches `App::error` and is still retained.

`after_insert` in `src/app.rs` remained the second half. It hard-coded `.`/`:`
for completion and `(`/`,`/`)` for signature help — the only place in the tree
those characters appeared — so a server was asked on one language's syntax
whatever its own handshake said. a388d4c added `completion_triggers`,
`signature_triggers`, and `signature_retriggers` to `Capabilities`, read in
`from_server` from `CompletionOptions::trigger_characters` and
`SignatureHelpOptions::trigger_characters`/`retrigger_characters`. A
`trigger_characters` helper drops entries that are not exactly one character,
since a keystroke can never match one, rather than half-matching their first.
`Capabilities` gave up `Copy` for the three `Vec` fields, so `supports` takes
`&self` and the `Ready` event clones. `after_insert` resolves both questions
once through a new `App::active_server_capabilities` — which
`has_language_server` is now expressed in terms of — and hands the signature
half to `after_insert_signature`. Path and word completion keep their existing
precedence over the language triggers.

Two deviations from what the report described. First, a server that advertises
a provider but names no trigger characters is given Runyte's own `.`/`:` and
`(`/`,` rather than nothing: a strict reading of the specification would leave
signature help unreachable, since Runyte binds no explicit command for it.
Second, the report asked only that the advertised characters drive when Runyte
asks; a388d4c also honours `retriggerCharacters` while a popup is showing,
which sends a request on the `)` that Runyte previously answered by clearing
the popup locally. That was wrong for nested calls: in `f(g(a), b)` the inner
`)` should return to `f`'s signature, and only the server knows which call the
caret is inside. The local clear survives as the fallback for a server that
names `)` neither way, so a popup can never be left open over a call that has
ended.

Checked against real servers rather than assumed, and the mechanism is not the
one the change was designed around. No server in the Docker matrix advertises
`retriggerCharacters` at all. What several do is list the closing `)` among
ordinary `triggerCharacters`, which reaches the same behavior through the
trigger path — and listing it is still not the same as answering after one:

    server                      signature help  asked on          answers after `)`
    clangd                      advertised      ( ) , < > { }     yes
    typescript-language-server  advertised      ( ) , <           yes
    Pyright                     advertised      ( ) ,             no
    rust-analyzer               advertised      ( , <             does not ask
    gopls                       advertised      ( ,               does not ask
    Marksman                    not advertised  —                 —
    sourcekit-lsp               not advertised  —                 —

So the nested-call improvement lands for clangd and
typescript-language-server. Pyright names `)` and then answers nothing at that
caret, so the popup closes exactly as the local clear used to close it, at the
cost of one round trip. rust-analyzer and gopls name `)` neither way and keep
the local clear unchanged. The retrigger path itself is exercised only by the
editor's own tests.

The two servers that advertise no signature help — Marksman and sourcekit-lsp
— are the case the report was written about: before the gate, every `(` and
every `,` typed in a Swift or Markdown buffer produced a retained `Method not
found`.

That second deviation was incomplete as first written. The specification
couples `retriggerCharacters` to the client capability
`textDocument.signatureHelp.contextSupport`, and a server that receives no
`SignatureHelpContext` cannot tell a retrigger from a fresh invocation — so a
compliant one was entitled to omit its retrigger characters entirely, or to
read the closing `)` as the start of a new request, which is precisely the
nested-call behavior the change claimed. A follow-up commit advertises
`contextSupport` in `client_capabilities` and carries a new
`lsp::SignatureContext` on `RequestKind::SignatureHelp`, which
`request_payload` serializes through `lsp_types::SignatureHelpContext` as
`triggerKind`, `triggerCharacter`, and `isRetrigger`. `after_insert_signature`
fills it from the character typed and from whether a popup was showing.

Covered by `a_server_that_never_advertised_signature_help_never_gets_the_request`
and `a_method_not_found_from_an_advertised_capability_is_still_a_retained_error`
in `src/app.rs`, joined there by
`completion_is_asked_for_on_the_servers_own_trigger_characters`,
`signature_help_retriggers_on_the_closing_delimiter_the_server_named`, and
`a_server_that_named_no_retrigger_character_still_closes_the_popup_locally`;
and in `src/lsp/mod.rs` by
`advertised_trigger_characters_replace_runytes_own`,
`an_advertised_provider_without_a_list_keeps_runytes_defaults`,
`a_multi_character_trigger_entry_is_dropped_rather_than_half_matched`, and
`a_provider_that_was_never_advertised_triggers_nothing`, which read the
capabilities from the wire shape so they exercise the same deserialization a
handshake does. The protocol context is covered in `src/lsp/mod.rs` by
`a_signature_request_carries_the_context_its_retrigger_needs`, which asserts
the serialized request parameters, and
`the_client_opts_into_signature_help_context`; the editor end of it is
asserted inside
`signature_help_retriggers_on_the_closing_delimiter_the_server_named`.

The opt-in Docker matrix in `tests/lsp/run.sh` covers the same ground against
seven real servers. Its `smoke` run now takes the advertised capabilities off
`LspEvent::Ready` and derives what it expects from them: a server that
advertises signature help must have a fixture probing it and must name an
opening `(`, and one that does not must have no probe, which is what makes
Marksman and sourcekit-lsp assert the gate rather than merely skip it. Each
fixture carries an unnested `pair(1, 2)` for the opening caret and a
`pair(wrap(1), 2)` for the caret after a nested call, with
`answers_after_nested_call` recording which servers answer there. The opening
caret is deliberately not the nested call's: gopls binds a caret sitting on
`wrap`'s first character to the inner call and answers for `wrap`.

Known limitation: no server in the matrix advertises `retriggerCharacters`,
so that branch of the gate has no real-server coverage; the matrix reaches the
closing-delimiter behavior through `triggerCharacters` instead. A server that
advertises a provider without naming trigger
characters gets Runyte's defaults rather than silence, so against such a
server the editor still asks on characters its author did not choose. The
signature context omits `activeSignatureHelp`, which is optional: the editor
keeps only the signature lines it rendered rather than the server's own value
to echo back, so a server that would use it to hold the active signature
steady across a retrigger does not get the chance. Completion sends no context
at all — `completion.contextSupport` stays `false`, so a server is not told
which character asked for it — and nothing consults
`completionProvider.allCommitCharacters`.

## Report

Runyte sends every language-server request to every server regardless of what
that server said it could do, so a server answers `Method not found` to
anything it does not implement and each refusal becomes a retained error
notification:

```
ERROR · 2026-08-18 09:18:46 · Runyte · Action failed · repeated 4 times
signature help: Method not found
```

The handshake discards the answer. `finish_initialize` in `src/lsp/mod.rs`
deserializes the server's `InitializeResult` and reads two fields out of
`capabilities` — `position_encoding` and `text_document_sync` — plus
`server_info.name`. Everything else the server advertised, including
`signature_help_provider`, `hover_provider`, `completion_provider`,
`rename_provider`, and `code_action_provider`, is dropped when `initialized`
goes out of scope. `struct Server` has no field to hold it, so no capability
state exists anywhere in the process.

Nothing can be gated as a result. `RequestKind` has fifteen variants and each
is sent whenever the editor wants it, because there is nothing to check first.
A server that does not implement one answers JSON-RPC `-32601`, which becomes
`Response::Failed`, and `apply_lsp_response` in `src/app.rs` turns any failure
into `self.error(reason)` — retained at `ERROR` severity and counted on the
status line.

Most of these requests are made because a key was pressed: `gd` asks for a
definition, `Space l r` asks for a rename. An unsupported one then costs one
error per press, which is self-limiting because the user stops pressing it.

Signature help is not one of those. It is sent as a side effect of typing:

```rust
match character {
    '.' | ':' => self.lsp_completion(),
    '(' | ',' => self.lsp_signature(),
    ')' => self.signature = None,
    _ => {}
}
```

Against a server without `signatureHelpProvider`, every function call produces
an error and every argument separator inside it produces another, so `f(a, b,
c)` produces four. This is the only unbounded producer of error notifications
in the editor, and it is why the unread count returns within seconds of being
acknowledged. Completion on `.` and `:` has the same exposure against a server
without `completionProvider`.

The server's advertised capabilities should be retained from the handshake and
consulted before a request is sent. A request the server does not support
should be a silent no-op rather than a failure: the editor asked for something
optional and the answer is that this server does not offer it, which is not an
error condition and is not worth retaining. This is also a round trip per
trigger character that is currently paid to a server that will always refuse.

A `Method not found` from a server that did advertise the capability is a
protocol violation and must remain a visible error. Only the requests that were
never supported in the first place become silent.

The capability shapes are not uniform, and the gate has to read each one as the
specification defines it rather than as a single boolean. Some are a plain
`Option`, some are a boolean-or-options union, and some carry the trigger
characters that ought to drive when Runyte asks at all — a server that lists
its own signature-help trigger characters is describing exactly the `(` and `,`
behavior currently hard-coded in `src/app.rs`.
