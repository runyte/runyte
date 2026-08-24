---
title: "A standalone Shift event cancelled the Space W binding"
status: resolved
reported: 2026-08-24
resolved: 2026-08-24
legacy_commit: b4852c6
---

## Resolution

Commit `b4852c6` (`Ignore standalone modifier input events`) corrected
`tui::input::convert_key_event`. Enhanced keyboard reporting emits a physical
modifier press separately from the character typed while that modifier is
held, but the conversion function was admitting standalone modifier presses
and repeats as editor keystrokes. Pressing Shift after the pending `Space`
prefix therefore sent `Shift-Modifier(LeftShift)` into the grammar, which
rejected the sequence and cleared it before `W` arrived.

Standalone modifier reports now return no editor input, just as modifier
releases already did. The filter covers every standalone modifier rather than
special-casing Shift because none carries editor intent on its own and each
could otherwise interrupt a pending sequence. The owned key-code conversion
remains exhaustive, and the shifted `W` keystroke still reaches binding
canonicalization with its character and modifier intact, so the registered
`Space W` command needs no keymap change.

Coverage lives in `src/tui/input.rs`:
`tests::standalone_modifier_events_do_not_become_editor_input` checks every
owned modifier across press, repeat, and release events, and
`tests::standalone_shift_does_not_enter_a_space_uppercase_sequence` checks that
the reported `Space`, standalone Left Shift, shifted `W` event stream becomes
exactly the two keys in the binding.

## Report

After a recent commit, using the `Space W` keybinding produced
`No keybindings: Space Shift-Modifier(LeftShift)` instead of opening the
persistent-session manager. The expected behavior was for `Space W` to remain
usable.
