---
title: "Opening a line with o/O discards indentation"
status: resolved
reported: 2026-09-29
resolved: 2026-09-29
commit: 3371da0
---

## Resolution

3371da0 (`Preserve exact indentation when opening lines above or below`)
changes `App::open_line`, which previously inserted only a line terminator.
Each distinct source row now contributes its exact leading tabs/spaces; the
new caret lands after that prefix, accounting for all preceding insertions in
character offsets. Opening above places the prefix before the new terminator;
opening below places it after. Line endings and the Insert undo group survive.
This preserves indentation with smart newline off too; syntax-driven extra
levels and list continuation remain Enter behavior.

Regression tests in `src/app/tests/editing.rs`:
`open_lines_preserve_exact_indentation_and_undo_the_insert_session`,
`open_lines_preserve_crlf_and_deduplicate_selected_rows`, and
`open_line_uses_the_surrounding_crlf_style_as_one_undo_group`.

## Report

[Issue #8](https://github.com/runyte/runyte/issues/8) reports that opening a line
with `o` or `O` starts at column zero even inside an indented block. The new
line should preserve the surrounding indentation, as Enter does.

Reproduction: place the caret on an indented line, press `o` or `O`, and type.
The text begins at column zero instead of after the inherited indentation.
The report does not specify syntax-driven extra levels or list continuation.
