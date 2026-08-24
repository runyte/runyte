---
title: "Terminal Normal mode has no directly usable copy surface"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: c051c83
---

## Resolution

Commit `c051c83` (`Add terminal Normal copy mode`) made Terminal Normal mode
enter the existing immutable review surface immediately. The review machinery
already had character, word, line, and vertical motion plus selection and
copying, but `App::execute_terminal_command` only routed those motions after a
search had created review state. Entering Normal therefore left no caret to
move, `v` was refused, and `j`/`k` only scrolled the live viewport.

`TerminalSession::begin_review` now captures the bounded retained output when
terminal input is left and places a visible caret at the nearest review
character to the child's captured cursor. Normal motions move that caret;
`v` enters Select mode and makes the same motions extend the range. A bare
caret copies its character, while an extended range copies its exact text,
including line breaks. The review caret also supplies the terminal status
position, and vertical motion uses terminal-cell columns so double-width text
does not shift the destination.

Runyte deliberately keeps its own selection-first keys instead of cloning the
tmux gesture literally: `Ctrl-\` or `Ctrl-w` leaves terminal input, `v` starts
selection, `y` copies to the unnamed register, and `Space c y` copies to the
system clipboard. The snapshot remains non-editable because terminal cells
are the child's rendered output rather than a Runyte text buffer.

Tests covering the behavior are
`normal_mode_has_a_movable_caret_and_selects_and_copies_terminal_text` in
`tests/terminal.rs`, and
`entering_review_places_a_visible_caret_at_the_child_cursor`,
`review_copy_preserves_unicode_and_line_breaks_and_motions_replace_or_extend`,
and `vertical_review_motion_preserves_terminal_cell_columns` in
`src/terminal/mod.rs`.

Known limitation: an alternate-screen program exposes only the visible screen
it had when review began, because alternate-screen history does not exist. New
child output continues behind an immutable review snapshot until `i` returns
to terminal input.

A later terminal-mode correction separated live Normal from review. The first
`Ctrl-\` now leaves Insert without capturing output, while the second
`Ctrl-\` or the first review operation creates the snapshot described above.
This preserves the copy surface while ensuring that leaving terminal input or
moving panes does not freeze or gray the terminal as a side effect. Coverage
lives in `tests/terminal.rs` test
`control_backslash_steps_from_terminal_input_through_normal_to_review`.

## Report

When a terminal pane is switched to Normal mode, its output should support a
movable cursor, text selection, and copying, comparable to `Ctrl-b [` copy mode
in tmux.
