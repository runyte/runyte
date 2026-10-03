`Ctrl-u` in Insert mode does not delete the full prefix requested by multiple
cursors on the same line. Each cursor creates a deletion from the line start
to its head. Transaction normalization sorts equal-start changes by end offset
and drops later overlapping changes, so only the shortest prefix is deleted.

For `abcdef` with Insert carets at character offsets 2 and 5, `Ctrl-u` leaves
`cdef`; all requested deletions together should leave `f`. The command should
union overlapping deletion spans before creating its transaction, preserve
other lines and CRLF terminators, map the carets correctly, and remain one undo
step.

Reproduce through a multi-selection followed by Insert mode and `Ctrl-u`, or
through the semantic `DeleteToLineStart` command with two carets on one line.
