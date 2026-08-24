---
title: "Space commands cannot be repeated"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: b0c5309
---

## Resolution

Commit b0c5309 (`Repeat the last Space command`) added `Space Space` to repeat
the last successfully invoked command reached through an actual `Space …`
binding. `RunyteGrammar::translate_modal` and `VimGrammar::translate_namespace`
previously reduced a resolved binding to a semantic invocation without keeping
its input provenance, so `App::apply_editor_intent` could not distinguish a
Space command from the same command reached through a `Ctrl-w` compatibility
alias or another surface.

`GrammarOutput` now identifies commands resolved from actual Space sequences,
and `App` retains the successful semantic invocation rather than raw keystrokes
or original coordinates. Repetition therefore applies the command to current
editor state, does not replace its own history, and does not lose the previous
successful command when a later Space command is unavailable or fails. Both
the Runyte and Vim grammars use the same application-owned history.

The proposed Vim-style `.` edit repeat was deliberately not added. The issue
was resolved only for Space command repetition, as requested in the follow-up;
text-edit recipe replay has separate semantics and is outside this change.

Tests covering the behavior are:

- `space_space_is_the_exact_repeat_command_in_both_modal_modes` in
  `src/keymap.rs`;
- `both_grammars_mark_only_actual_space_namespace_commands_for_history` in
  `src/input_grammar.rs`;
- `space_space_repeats_the_last_successful_space_invocation_in_both_grammars`
  in `src/app.rs`;
- `command_inventory_classifies_every_command_and_current_binding` in
  `src/app.rs`.

The change also passes `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`.

Known limitation: Space command history is process-local and starts empty each
time Runyte launches.

## Report

There was no keybinding for repeating the last action. `Space Space` should
repeat the last `Space`-type keybinding.

Whether it should also repeat text edits, as `.` does in Vim, was left open —
possibly `Space Space` for command repeat and `.` for text-edit repeat.
