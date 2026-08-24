---
title: "Pressing d on an empty line does nothing"
status: resolved
reported: 2026-07-31
resolved: 2026-07-31
legacy_commit: 21f633e
---

## Resolution

Fixed in commit `21f633e`, "Delete empty lines with d".

`operative_span` in `src/app.rs` always stopped at the visible end of a row so
that ordinary character operations would not consume its line terminator. On
an empty row, however, its start and visible end are the same offset. That
turned `d` into an empty transaction even though Helix treats the line ending
as the selected character on that row.

The span calculation now gives an empty row its line terminator. It consumes
the following terminator for an empty row before other text and the preceding
terminator for the final empty row, where there is no following character.
This keeps the edit on the normal transactional deletion path, including
register yanking, multi-pane selection mapping, and single-step undo. A truly
empty one-line buffer still has no terminator and remains unchanged.

Covered by `delete_on_an_empty_row_removes_its_line_ending` and
`delete_in_a_truly_empty_buffer_remains_a_no_op` in `src/app.rs`.

## Report

Pressing `d` on an empty line did nothing. The line should be removed, as it
is in Helix.
