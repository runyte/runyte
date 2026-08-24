---
title: "Terminal review lacks line and multi-selection commands"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: 2ccf9d4
---

## Resolution

Commit `2ccf9d4` (`Add terminal review selection commands`) extended terminal
review from one `Range` to a normalized `Selection`. The first terminal copy
mode implementation routed `v` and ordinary motions, but
`App::execute_terminal_command` had no terminal behavior for the existing
`SelectLine`, `SelectLineUp`, `CopySelectionDown`, or `CopySelectionUp`
commands. Those commands therefore reached the terminal refusal path, and the
single review range could not represent several carets even if they had been
routed.

`TerminalSession` now applies review motions to every range, renders secondary
carets and ranges, reports the actual selection count, and copies all operative
spans in normalized order with newlines between them. `x` and `X` reuse the
editor's transient line-selection lifecycle: the first press snaps every range
to its line and repeated presses walk the moving edge down or up. A linewise
yank retains its terminating newline. `C` and `Alt-C` add carets in the
requested direction at the same terminal-cell column and skip short rows,
including rows ending exactly at that column. This keeps wide glyphs aligned
without inventing text in the immutable snapshot. Existing `v` selection now
extends every review range.

Tests covering the behavior are
`terminal_review_line_and_multi_selection_commands_copy_together` and
`normal_mode_has_a_movable_caret_and_selects_and_copies_terminal_text` in
`tests/terminal.rs`, and
`review_line_selection_snaps_then_walks_both_directions` and
`review_copy_selection_skips_short_rows_and_line_selects_every_caret` in
`src/terminal/mod.rs`.

Known limitation: the padded `V`/`Alt-V` multi-caret commands remain refused
in terminal review because padding would require changing or inventing child
output in an immutable snapshot.

## Report

Terminal Normal review mode should support `v`, `x`/`X`, and `C`/`Alt-C` for
flexible selection construction and copying.
