---
title: "Session listing paths can corrupt terminal table rows"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 87b74ee
---

## Resolution

Commit `87b74ee` (`fix(session): escape control characters in CLI table cells`).

format_session_table now escapes control characters in every displayed cell before calculating widths. Normal Unicode remains readable, while newline, tab, carriage return, and ESC become visible text. The underlying workspace rows, paths, and identifiers are unchanged, so listing presentation cannot change selection or connection identity.

Coverage: session_table_escapes_control_characters_without_changing_workspace_identity in src/tui/tests/session_table.rs checks one output row per workspace, absence of raw controls, readable Unicode, visible escapes, and unchanged identity. The regression produced extra rows before the fix and passes afterward.

## Report

`--session-list` writes workspace names and directory paths directly into its
terminal table. A valid Unix directory containing a newline creates additional
rows, and a directory containing an escape character can emit terminal control
sequences. Width calculation also treats these bytes as presentation content,
so one filesystem name can corrupt the rest of the listing.

The session table must show each workspace on exactly one row and render
control characters as visible escaped text. Ordinary Unicode names and paths
must remain readable, and raw workspace identities used for selecting and
connecting must remain unchanged. A regression can construct a workspace row
whose directory contains newline, tab, carriage return, and ESC, then inspect
the formatted table without starting a terminal or creating such a workspace.
