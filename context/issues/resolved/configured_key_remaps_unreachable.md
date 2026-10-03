---
title: "Configured key remaps can advertise unreachable commands"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 71be29b
---

## Resolution

Commit `71be29b` (`fix(keymap): reject remaps consumed by the input grammar`).

The effective-scope validator now rejects modal sequences beginning with a
count key and sequences containing prefix-cancellation keys after their first
key. Previously only direct `keys.bind` assignments checked those reservations;
named prefixes and `keys.rebind` could remove reachable defaults and replace
them with bindings consumed by the grammar. The existing bounded rollback
retains defaults for rejected rules while keeping independent valid rules.
Insert-mode bindings and directory-tree numbered commands keep their scope
semantics. Tab prefixes remain rejected by the existing exact/prefix collision
with the built-in context-action key.

A follow-up review also found that the frontend's key-discovery state consumes
`Ctrl-n` and `Ctrl-p` while a prefix is pending. Effective validation now rejects
those continuation keys in Normal and Select and under the window namespace
in Insert and Replace, preserving the hint scrolling controls and retaining
reachable default bindings. Arbitrary Insert/Replace prefixes bypass hints
and keep their ordinary `Ctrl-n`/`Ctrl-p` continuations.

`grammar_reserved_remaps_restore_reachable_defaults` in
`src/keymap/configured.rs` passed. It checks rejected leader/rebind targets,
actual Normal and Select dispatch after rollback, independent valid mappings,
and Insert-mode Tab. The parent reviewer checked the validator against
`RunyteGrammar::translate_modal` and the remapping admission rules.
`hint_scroll_keys_cannot_replace_prefix_continuations` in
`src/keymap/configured.rs` covers both remaps and direct configured actions.

## Report

Configured key remappings can be accepted even when the input grammar cannot
execute them. For example, `keys: {leader: '1'}` moves the leader menu onto a
decimal count key, and `keys: {rebind: {'Space e': '2 e'}}` advertises an explorer
binding whose first key is consumed as a count. A target such as `F12 Esc e` or
`F12 Backspace e` is also accepted, although the second key cancels or removes
the pending prefix.

The compiler checks structural keymap collisions but does not check these
grammar reservations for named prefixes and `keys.rebind`. `keys.bind` already
rejects comparable assignments. The accepted remappings remove their working
default bindings and cause help and hints to advertise unreachable commands.

Reject unreachable mappings non-fatally, retain the affected defaults, and
preserve independent valid rules. Check effective modes and scopes so Insert
mode's ordinary Tab and the directory tree's numbered pane keys remain valid.
Regression coverage should exercise both compiled lookup and actual grammar
dispatch after rejection.

Sequences such as `F12 Ctrl-n` and `Ctrl-w Ctrl-p` are also unreachable in the
frontend because key discovery consumes their final key to scroll hints. This
applies to configured direct actions as well as remaps, including Insert/Replace
window-prefix continuations.
