---
title: "Jumplist traversal cursor was not rebased after earlier entries were removed"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: 16d4375
---

## Resolution

Commit `16d4375` (`Rebase jumplist cursor after entry removal`) corrected
`JumpList::forget` and `JumpList::retire_buffer`. Both functions compacted the
entry list and only clamped `current` to its new length, so removing an entry
before a surviving mid-history position left the numeric cursor one place
ahead of the logical location on screen.

Both removal paths now use `JumpList::retain_rebasing_current`, which records
the old cursor and subtracts exactly the removed entries whose old indices
preceded it. This keeps a surviving current position attached to the same jump,
preserves the past-newest sentinel when `current` equals the old list length,
and does not count terminal entries that `retire_buffer` retains with a
replacement backing buffer. No keybinding changed, and the deliberate
full-history behavior in which initial backward traversal can temporarily
retain 31 entries remains intact.

Regression coverage is in `src/jumplist.rs`:

- `forgetting_an_earlier_entry_rebases_mid_history_traversal`;
- `retiring_an_earlier_entry_rebases_mid_history_traversal`.

## Report

`JumpList::forget` and `JumpList::retire_buffer` remove entries and then clamp
`current` to the new list length. They do not account for removed entries that
were positioned before the current traversal location.

This can detach the numeric cursor from the logical location being displayed.
For example, given remembered locations `A`, `B`, and `C`, starting backward
traversal from `D` lands on `C` and records `D` for forward traversal. If `A`
is then removed, `C` shifts one index toward the start but `current` does not.
The next backward step can revisit `C` instead of landing on `B`, and a forward
step from `C` can report the end of history instead of returning to `D`.

The same underlying list serves `Ctrl-o` and `Ctrl-i` for position-by-position
navigation and `Alt-o` and `Alt-i` for cross-buffer or cross-terminal-surface
navigation, so either command pair can expose the misalignment. Entry removal
occurs when a buffer is retired and when a directory buffer is retargeted or
reloaded with content whose old row offsets are no longer meaningful.

After entries are removed, traversal must remain attached to the same logical
position whenever that position survives. Backward and forward navigation
must then visit the neighboring surviving entries without revisiting the
current location, skipping a neighbor, or incorrectly reporting an end.

Focused `src/jumplist.rs` regressions cover removal before a mid-history cursor
through both `forget` and `retire_buffer`, including backward and forward
navigation. The unit boundary establishes the command-visible behavior, so no
application-level state-transition test is needed.

No keybinding or the deliberate full-history behavior in which the first
backward step records the current location even when that temporarily
increases a 30-entry jumplist to 31 entries changes as part of the fix.
