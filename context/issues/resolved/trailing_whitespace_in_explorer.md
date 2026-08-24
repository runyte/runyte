---
title: "Trailing whitespace in explorer listings is treated as a filesystem move"
status: resolved
reported: 2026-08-10
resolved: 2026-08-10
legacy_commit: cf6cda7
---

## Resolution

Commit cf6cda7 (`Ignore trailing whitespace in explorer plans`) fixed the
directory projection parser. `DirectoryBuffer::plan` previously considered a
whitespace-only row to be a new filesystem entry, while `parse_line` preserved
spaces after an existing label. In particular, adding a space after `context/`
made the directory look like it was being moved beneath itself and reached the
filesystem plan's self-move guard.

Directory planning now ignores whitespace-only rows and removes trailing
whitespace before interpreting each entry. When those edits produce an empty
plan, `App::write_buffer` reloads the directory buffer instead of merely
marking its edited text as saved. This restores the canonical listing, clears
the modified marker, and does not open filesystem confirmation.

The behavior is covered by
`tests/directory_buffer.rs::writing_whitespace_only_explorer_edits_refreshes_the_listing`,
which adds trailing spaces to directory and file rows, appends whitespace-only
rows, writes the explorer, and verifies that no plan is offered and the exact
directory projection is restored.

Known limitation: filesystem names ending in whitespace cannot be expressed
through an editable directory buffer because trailing whitespace is now
deliberately presentation-only there.

## Report

Adding spaces at the end of entries in the explorer displayed the directory as
changed with `[+]`. Writing the directory buffer then showed the red error
`cannot move context inside itself`, whose meaning was unclear. Whitespace and
newline-only edits were expected not to trigger filesystem changes, and `:w`
was expected to remove that whitespace and refresh the directory view.
