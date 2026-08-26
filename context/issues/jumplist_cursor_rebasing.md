# Jumplist traversal cursor is not rebased after earlier entries are removed

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

Add focused `src/jumplist.rs` regressions for removal before a mid-history
cursor through both `forget` and `retire_buffer`, covering backward and forward
navigation. Add an application-level state-transition test only if the unit
boundary cannot establish the command-visible behavior.

Do not change any keybinding or the deliberate full-history behavior in which
the first backward step records the current location even when that
temporarily increases a 30-entry jumplist to 31 entries.
