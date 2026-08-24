---
title: Delimiter text objects select one character beyond their structural bounds
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: 7c874f0
---

## Resolution

Fixed in commit `7c874f0`, "Fix delimiter text object selection bounds".

`App::select_delimiter` installed the correct half-open range returned by the
syntax layer, but left the pane tagged with Runyte's inclusive selection
semantics. Rendering and editing therefore treated the range's exclusive end
as one more selected character: the closing delimiter for an inside object,
or the character following the closing delimiter for an around object.

Delimiter selections are now marked with `SelectionSemantics::HalfOpen`, the
coordinate model their syntax ranges already use. This keeps the command
provider-neutral across parentheses, square brackets, braces, angle brackets,
and quotes without changing the structural delimiter resolver.

Covered by
`delimiter_text_objects_select_nested_pairs_through_headless_commands` in
`tests/headless_editor.rs`, which deletes inside and around selections and
checks that the delimiters and following character, respectively, survive.

## Report

`Space x i (` and the rest of that family selected the text inside matching
brackets plus the closing bracket. `Space x a (` selected the text within the
brackets, both brackets, and one further character.

`Space x i` should select only the text within the delimiters, and
`Space x a` only that text plus the delimiters themselves, with no trailing
characters.
