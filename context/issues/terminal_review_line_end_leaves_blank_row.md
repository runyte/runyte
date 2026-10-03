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
