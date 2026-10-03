---
title: "Same-line cursors leave requested prefix text undeleted"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 3ff1c7f
---

## Resolution

Commit `3ff1c7f` (`fix(editing): merge overlapping line-prefix deletions`).

`App::delete_to_line_start` submitted overlapping equal-start deletions directly
to transaction normalization, which retained the shortest change. It now unions
the requested spans through the existing CRLF-safe deletion helper before
building one transaction. Every cursor contributes its full requested prefix,
and caret mapping and undo continue through the ordinary transaction boundary.

Coverage: `delete_to_line_start_unions_same_line_carets_and_undoes_once` in
`src/app/tests/editing.rs` sends Insert-mode `Ctrl-u` across four carets on two
CRLF-separated lines and verifies resulting text, caret positions, and one-step
undo. The regression failed before the change and passes after it.

## Report

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
