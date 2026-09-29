# Whole-line extension above

The follow-up discussion on [issue #4](https://github.com/runyte/runyte/issues/4)
requests Helix's `extend_line_above` and `extend_line_below` commands as
configurable actions. Selection undo/redo is already implemented on `Alt-u`
and `Alt-U` (`Alt-Shift-u`), so `X` no longer needs to retract the edge walked
by `x`. The initial follow-up proposed keeping the default; the accepted
decision is to make `extend-line-above` the default `X` and retain the old
`select-line-up` action for configuration.

Reproduction: start on a middle line and press `x x X`. The existing edge walk
removes the second selected line; extension above should preserve both lines
and add the line before them. Partial ranges should first expand to whole
lines. Counts, multiple selections, empty lines and selection history must
remain supported. General action-binding configuration is a separate change.
