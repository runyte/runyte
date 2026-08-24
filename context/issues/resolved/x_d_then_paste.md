---
title: "Cutting a line selection lost its linewise paste semantics"
status: resolved
reported: 2026-08-11
resolved: 2026-08-11
legacy_commit: 2aeadce
---

## Resolution

Commit `2aeadce` (`fix linewise cuts from transient selections`) fixed
`App::delete_selection_or_char`, which only recognized Vim linewise selections
and discarded the transient line-selection state created by `x` and `X`.
Deletion now receives that state for `d`, writes a linewise register, and
removes the selected rows together with their line boundary. Deleting a final
unterminated row consumes its preceding boundary so it does not leave an empty
line. `App::paste_register` also supplies the missing separator when a linewise
register is pasted after an unterminated final row.

An explicit `v` selection remains characterwise even when it covers text on
only one line. The related `x c` behavior was deliberately left unchanged: the
report concerned yank and cut, and change must preserve an insertion line.

Covered by `app::tests::x_delete_pastes_whole_lines_but_v_delete_remains_characterwise`
and `app::tests::x_and_x_yanks_paste_whole_lines_but_v_yanks_remain_characterwise`
in `src/app.rs`. Explorer move behavior remains covered by
`app::tests::explorer_delete_and_paste_moves_across_panes_on_write` in
`src/app.rs`.

## Report

Selecting a line with `x`, yanking it with `y`, and then pasting it with `p`
correctly pasted an entire line. Selecting a line with `x`, cutting it with
`d`, and then pasting it with `p` instead pasted the text without line
boundaries at the cursor, including in the middle of a sentence.

Selections created by `x` and then yanked or cut with `y` or `d` were expected
to include their line boundaries so that pasting created a complete line.
Manual single-line selections created with `v` and motion keys were expected
to remain characterwise without added line boundaries.
