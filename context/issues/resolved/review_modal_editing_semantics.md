---
title: "Modal editing operations violated selection, display-column, and line-ending invariants"
status: resolved
reported: 2026-08-25
resolved: 2026-08-26
commit: b9e9419
---

## Resolution

Commit `b9e9419` (`Harden modal editing semantics`) corrected the confirmed
selection and text-integrity defects without changing Runyte's keymap or motion
semantics. `line_register` and selection-scoped trailing-whitespace trimming
now hold back the exclusive next row of a half-open pointer or Vim-linewise
selection. Insert-mode tab stops and selection alignment now measure terminal
display cells, including tab expansion and wide characters, rather than raw
character columns. The user guide and Helix divergence register describe the
rightmost alignment target as a display column.

The editing path now treats CRLF as one line terminator. Newline insertion,
`o`/`O`, Backspace, Delete, replace-character, linewise yank/delete/paste, Vim
visual-line change, hard wrap, and reflow preserve complete terminators and the
nearby or registered line-ending style. Expanded deletion spans are unioned
before transaction construction so overlapping multi-selection corrections
cannot drop part of a requested deletion. Vim line change uses the same exact
line-register construction as yank and delete, retains the complete existing
terminator for the replacement row, and groups the deletion with subsequent
Insert-mode input for one-step undo. Syntax-assisted newline indentation now
queries the newline token of either an LF or CRLF row.

Regression coverage is in these tests:

- `src/app/tests/editing.rs`:
  `line_commands_hold_back_a_half_open_selection_end_row`,
  `replace_preserves_crlf_terminators_inside_a_selection`,
  `linewise_delete_and_paste_keep_crlf_terminators_atomic`,
  `open_line_uses_the_surrounding_crlf_style_as_one_undo_group`, and
  `tab_stops_and_selection_alignment_use_display_columns`;
- `src/app/tests/editing_and_buffers.rs`:
  `smart_newline_preserves_crlf_and_uses_its_syntax_indent` and the CRLF case
  in `disabled_smart_newline_keeps_leading_indent_without_list_alignment`;
- `src/app/tests/search_and_pickers.rs`:
  `insert_backspace_and_delete_treat_crlf_as_one_line_break`;
- `src/app/tests/commands.rs`:
  `vim_visual_line_change_preserves_crlf_registers_and_undo_grouping`;
- `src/wrap.rs`: the CRLF case in
  `hard_wrap_uses_word_boundaries_and_preserves_existing_newlines` and
  `reflow_preserves_consistent_crlf_line_endings`.

Known limitation: `JumpList::forget` and `JumpList::retire_buffer` clamp
`current` after removing entries but do not subtract entries removed before it,
so retirement during mid-history traversal can skip or revisit the wrong
neighboring destination. The jumplist source and bindings were deliberately
left untouched; follow-up work is tracked in
`context/issues/jumplist_cursor_rebasing.md`.

## Report

Runyte's selection-first editing semantics require a focused hardening review.
The review is proactive rather than evidence that every operation is faulty;
changes are appropriate only for confirmed defects, and deliberate differences
from Helix remain authoritative.

The primary review boundary is `src/app/editing.rs`, `src/app/movement.rs`,
`src/structural_selection.rs`, `src/wrap.rs`, `src/table.rs`,
`src/jumplist.rs`, macro and register handling, and their tests. The relevant
invariants include counts, primary and secondary selections, overlapping
ranges, empty lines and documents, document ends, delete/change/yank/paste
symmetry, indentation, joining, case conversion, smart newline, structural
objects, syntax-unavailable fallbacks, visual-line movement, jumplist updates,
macro recording and replay, undo grouping, and Unicode. The current Runyte
keymap and its documented deviations are the behavior specification; the
Helix reference is a divergence register, not a target specification.

Confirmed selection defects occurred when a non-empty half-open range ended at
column zero of the following row. Whole-line yank (`Y`) and selection-scoped
trailing-whitespace trim (`_`) treated that exclusive endpoint as another
selected row. For example, a pointer selection from offset zero through
`line_to_offset(1)` could yank or trim row 1 even though it selected only row 0.

Confirmed display-column defects occurred in Insert-mode `Tab` and selection
alignment (`&` or `Space s a`). Both used character columns, so a caret after a
tab or a double-width Unicode character received the wrong number of padding
spaces. The expected target is the next configured display-cell tab stop for
Insert `Tab`, or the rightmost selection display column for alignment.

Confirmed line-ending defects occurred in CRLF buffers. Backspace, Delete, and
replace-character could leave only `\r` or `\n`; smart newline, disabled smart
newline, `o`/`O`, hard wrap, and reflow could introduce bare LF; and linewise
yank, delete, paste, and Vim visual-line change normalized register text or
deleted only one character of a terminator. Final unterminated rows exposed
additional delete/paste and register-symmetry failures. These operations must
keep CRLF atomic, preserve exact register terminators where text is transferred,
choose the surrounding style for newly created rows, and retain one-step undo
grouping for compound edits.

The review also covered editing and movement counts, multi-selection ordering
and overlap behavior, empty documents and rows, EOF operations, indentation,
joining, case conversion, structural selection and syntax fallbacks, wrap and
table behavior, visual-line movement, macro replay and registers, and Unicode.
No additional safely scoped defect was confirmed in those areas. Structural,
table, macro, register, and unchanged jumplist baseline suites remain green.

Jumplist inspection found two separate boundary behaviors. Starting backward
traversal records the current location even when the list already contains 30
entries, temporarily retaining 31 so forward traversal can return to its exact
origin. That behavior is deliberate and remains unchanged. Removing a buffer
while the traversal cursor is inside history removes matching entries without
rebasing the cursor by the number removed before it. That confirmed defect is
tracked separately in `context/issues/jumplist_cursor_rebasing.md`; it was not
changed as part of this review.
