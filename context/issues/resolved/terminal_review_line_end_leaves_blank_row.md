---
title: "Terminal review line-end motion leaves empty rows"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 5e051a1
---

## Resolution

Commit `5e051a1` (`fix(terminal): keep review line-end motion on empty rows`).

review_motion_target used text_end minus one even when a review row contained no characters. LineEnd now uses the existing review_line_last_offset helper, which clamps the result to the row start. Empty rows retain their caret and extending-selection endpoint, while nonempty rows still select the last character.

Coverage: review_line_end_stays_on_empty_rows in src/terminal/mod.rs covers middle and trailing empty rows, extending and nonextending motion, and a nonempty row. The regression failed before the fix; all 15 terminal review tests pass.

## Report

The line-end motion in terminal review moves a caret off an empty row onto
the preceding row's separator. `review_motion_target` computes `text_end - 1`
without clamping it to that row's start. Empty review rows have equal start
and end offsets, so this subtraction resolves to the previous row instead.

The `$`, End, and goto-line-end commands must leave a caret on its current
empty row, including when extending a selection. Nonempty rows should still
move to their final character.

Reproduce with terminal output containing `one`, an empty line, and `two`.
Enter review, move to the empty row, and invoke line end. The caret should
remain on the empty row instead of moving to the end of `one`.
