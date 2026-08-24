---
title: "Replacing a character with Space opened Space command hints"
status: resolved
reported: 2026-08-12
resolved: 2026-08-12
legacy_commit: 375a266
---

## Resolution

Commit `375a266` (`Treat replacement space as literal input`) fixed the
frontend key-hint dispatch. The standalone event loop and attached-host path
were sending every Normal- or Select-mode key to `KeyHintState` before the
input grammar saw it. Although `RunyteGrammar` had already recorded that `r`
owned the next character, the independent hint observer interpreted a Space
argument as the start of the application command tree.

`App::key_hint_mode` now withholds a registry mode while either active grammar
is waiting for a literal character, and both frontend paths use one helper
that clears hints and forwards such input directly to the grammar. This makes
the first Space in `r Space Space` replacement text and lets only the second
Space open command hints. The rule applies to every command whose grammar
state owns a character argument, so hint discovery cannot compete with
literal operands.

Runyte deliberately keeps the status mode as Normal while `r` waits. Replace
is a one-character operator argument with its own cancellation and dispatch
semantics, not an Insert-mode session; changing the label to `INS` would claim
that Insert bindings and arbitrary text input were active. The existing red
replacement heads continue to expose the temporary state without changing
the mode label.

Coverage:

- `replacement_space_is_not_observed_as_a_space_command_prefix` in
  `tests/key_hints.rs` exercises `r Space Space` through the public hint and
  editor boundaries, including the replacement, hidden first popup, retained
  Normal mode, and popup opened by the second Space.
- `attached_host_treats_replacement_space_as_character_input` in `src/main.rs`
  exercises the same sequence through the persistent-host frontend dispatch.

## Report

Pressing `r Space` to replace a character with a space immediately opened the
popup containing Space command helpers. The first Space should be consumed as
literal replacement input, as text is consumed after `r`, and should not open
that popup.

The cursor changed from blue to red after pressing `r`, but the bottom-left
mode name did not change because Insert mode was not entered. Changing the
mode label to `INS` until the replacement character was entered was suggested
as a potentially more consistent presentation; whether replacement-pending
state should be represented as Insert mode was left undecided.

The expected gesture is `r Space Space`: the first Space replaces the
character, and the second Space starts a Space command and opens its helpers.
