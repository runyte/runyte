---
title: "Macros are reachable only through the unmemorable Q and q keys"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: af2a67c
---

## Resolution

Fixed by af2a67c "Give macros a Space m namespace instead of Q and q".

`Q` and `q` were two bare `modal` bindings on `record-macro` and
`replay-macro`, and both are gone. The namespace the report asked for is now
five bindings under a `Space m` "Macros" namespace row, so the `Space` hints
name the whole vocabulary instead of leaving it to memory:

- `Space m m` → `record-default-macro`
- `Space m M` → `record-macro`, which takes the register as the next key
- `Space m r` → `replay-default-macro`
- `Space m R` → `replay-macro`, which takes the register as the next key
- `Space m l` → `list-macros`

The report did not say how a recording should end, and `Space m m` doing both
is the answer that needs nothing else remembered: `record_default_macro`
stops the open recording if there is one and otherwise starts one on
`DEFAULT_MACRO_REGISTER`, which is `@` — the register Helix and Vim already
spell macros with, and one no register key reaches by accident.

That toggle is the reason for the one non-obvious change. `Q` could stop a
recording because `RunyteGrammar::translate_modal` intercepted it before
dispatch, so the key never reached `handle_input`'s recorder. A three-key
sequence cannot be intercepted that way: `Space` and `m` are ordinary recorded
input by the time the second `m` resolves. So `handle_input` no longer appends
straight into the macro. Events go to `macro_staging` and move into the macro
only once `grammar.pending_sequence()` is empty — once their sequence has
resolved into something. `stop_macro_recording` clears the staging, which
drops precisely the keys that spelled the stop. Order is preserved because
staging only defers; nothing is reordered. The `Q` interception in the Runyte
grammar was deleted along with the binding.

Two smaller decisions. `start_macro_recording` now refuses a second recording
while one is open — `Space m M x` during a recording is an error naming
`Space m m`, not a silent replacement of what was being recorded. And
`Space m l` opens the shared `ListPicker` rather than printing a status line,
with the default macro sorted first, each row showing its input count and
whether it is the one recording, and Enter replaying the selected macro
through the new `ListAction::Macro`.

The Vim grammar is untouched in its own vocabulary: `q{a-z}`, `q` to stop, and
counted `@{a-z}` all still work, and `macro_stop_hint` still says `q` there.
But Vim delegates `Space …` to the shared keymap, and that path called
`BindingTarget::invocation()` with no character, which the command inventory
rejects as incomplete — `Space m M` under Vim would have failed the whole
input with `record-macro requires a character operand`. `translate_namespace`
now parks a `VimAwaiting::NamespaceCharacter` for any keymap command that
takes a character, the same way the Runyte grammar parks
`awaiting_character`. Vim still drops a count when it enters a Space
namespace, so `Space m R` there replays once and repetition stays on `2@a`.

Covered in `src/app.rs` by
`named_macros_record_stop_and_replay_through_the_macro_namespace`,
`the_default_macro_is_recorded_replayed_and_listed_from_one_namespace` (which
asserts the staged stop keys are not in the macro),
`the_old_macro_aliases_are_gone_and_a_second_recording_is_refused`,
`the_macro_namespace_awaits_its_register_under_the_vim_grammar_too`,
`an_empty_macro_list_says_so_instead_of_opening_an_empty_picker`, and
`literal_text_is_one_insert_transaction_and_one_macro_event`.

Known limitation: `Space m m` cannot be recorded *into* a macro, since it is
the sequence that ends one — the same trade `Q` made. Macros remain
process-local; nothing about them is written to disk, so `Space m l` is empty
in a fresh session. In-app help still does not list the namespace: the Normal
overview in `src/help.rs` is already at the row budget an 80×24 terminal can
show, so discovery goes through the `Space` hints and the README.

## Report

Macros could only be recorded through the `Q` and `q` keys, which were hard to
remember. Those aliases should be dropped in favour of a `Space m` namespace:

- `Space m` — macro namespace
- `Space m m` — record a new default macro
- `Space m M <key>` — record a new macro named `<key>`
- `Space m r` — replay the default macro
- `Space m R <key>` — replay the `<key>` macro
- `Space m l` — list all macros, named and default
