---
title: "C drifts its column when lines have different lengths"
status: resolved
reported: 2026-08-13
resolved: 2026-08-13
legacy_commit: 444c93f
---

## Resolution

Commit `444c93f` (`Skip lines with no character at the cursor column`) corrected
the row test in `App::copy_selection`. The command accepted any row satisfying
`line_len(row) >= position.col`, which admits a row that ends exactly at the
column even though no character sits there. Normal mode may not rest a caret on
a row's terminator, so the `clamp_offset` that built the new caret slid it one
column left onto the row's last character. Because each press reads its column
back from the caret it copies, that shifted column then seeded every caret
below: a single short row bent the rest of the column, including on rows long
enough to hold the original one. In the reported document, `- Three` is exactly
seven characters, accepted a caret at column 7, moved it to column 6, and the
carets on the following lines inherited column 6.

The test is now `line_len(row) > position.col`: a row qualifies only when the
column is occupied, not merely reachable. The target offset is in range by
construction, so the clamp was removed rather than left as a silent correction —
it was the mechanism that turned a wrong row into a wrong column. Rows that
cannot hold the column are skipped and the search continues to the next one, as
it already did for rows that were plainly too short.

Empty rows now fail the same test at column 0, where they previously always
matched. This follows from the reported rule rather than from the reported
symptom, and it is deliberate: a caret is no longer left on the blank lines
between paragraphs when a column of cursors is built to prefix them.
`copy-selection-down-padded` and its upward mirror on `V` and `Alt-V` are
unchanged and remain the commands that widen short rows to reach a column
instead of skipping them.

Coverage lives in `tests/selection.rs`, which drives real key dispatch:
`a_cursor_is_skipped_on_a_row_that_ends_at_the_column`,
`repeated_copies_hold_the_column_across_rows_that_cannot_take_it` — which
rebuilds the reported document, presses `C` three times, and asserts every caret
stays in column 7 — and `a_cursor_is_skipped_on_an_empty_row`. The existing
`a_cursor_is_skipped_on_rows_too_short_for_the_column` still covers the plainly
short row.

Known limitation: `C` matches on character columns, while the padded `V`
reaches a display column. On rows mixing tabs or double-width characters the two
commands therefore disagree about which column a caret is in, and `C` can place
carets that do not line up on screen.

## Report

`C` behaves inconsistently on lines of different length. It sometimes places the
new cursor on the line's last character and sometimes does not, and once a
cursor's position has shifted, the shifted position persists for the cursors
after it even when the lines below are longer than the preceding line.

In the reported document the carets produced by repeated `C` from column 7 of
`Some list:` landed as follows, with `- Two` and the empty lines passed over and
the column moving from 7 to 6 at `- Three`:

```
Some li|st:
- One a|bc
- Two
- Thre|e
Anothe|r:
2. Ora|nge
```

The expected behavior is that `C` places a cursor only on lines that have a
character at the original column. A line too short to have one is skipped.
`V` already exists for padded multicursors that reach the column regardless of
line length.
