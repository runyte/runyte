---
title: "Horizontal split has no Space shortcut to match Space v"
status: resolved
reported: 2026-07-31
resolved: 2026-07-31
legacy_commit: 4c66779
---

## Resolution

Fixed by 4c66779 "Add Space h as the horizontal split shortcut".

Nothing was broken; the shortcut had simply never been added. `Space v` has
mapped to `split-vertical` since V0, but the stacked split reached the
registry only through `Ctrl-w s` and `Space w s`. `Space s` had been Runyte's
stacked-split shortcut until V4 Phase 2 gave it back to Helix's document
symbol picker, and no two-key replacement was registered at the time. The fix
adds one `modal([Space, h], Command::SplitHorizontal)` binding in
`src/keymap.rs` next to the existing `Space v` row.

`Space h` deviates from stock Helix, which binds Space-mode `h` to
`select_references_to_symbol_under_cursor`. Runyte does not implement that
command under any binding, so nothing was displaced, and the deviation is
recorded in `context/reference/helix-keymap-v1.md` alongside the `Space v`
row it mirrors.

No dispatch changes were needed. `Space` is already a multi-key prefix, and
key lookup is a prefix tree over the whole typed sequence, so the top-level
`h` (move-left) binding is unaffected. Help and key hints read from the same
registry, so they picked the binding up without edits.

Tests: `space_splits_are_symmetric` in `src/keymap.rs` asserts that both
`Space v` and `Space h` resolve to their split commands, and the existing
`default_keymap_has_no_duplicate_sequences_per_mode` in the same file guards
against the new sequence colliding with another binding.

## Report

A `Space` shortcut existed for vertical split (`Space v`). An equivalent was
requested for horizontal split, proposed as `Space h`.
