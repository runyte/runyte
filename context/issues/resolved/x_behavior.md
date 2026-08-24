---
title: Line selection with x does not extend from an empty line
status: resolved
reported: 2026-07-30
resolved: 2026-07-30
legacy_commit: 99f8143
---

## Resolution

Fixed in commit `99f8143`, "Make x/X walk one line-selection edge in both
directions".

`select_line` in `src/app.rs` decided whether to extend by testing
`range.is_empty()`. A selected empty line *is* an empty range, because
`Buffer::row_end_offset(row, false)` collapses onto the line start when the
line has no characters, so the second `x` on an empty line re-selected the
same line forever. The command also forced `Mode::Select`, which is why `j`
and `k` afterwards extended as though `v` had been pressed.

What changed:

- `select_line` now works in row arithmetic rather than range emptiness. The
  first press snaps each range to the whole lines it already touches; every
  press after that moves the head row by one, so an empty line extends like
  any other.
- `X` is bound to a new `select-line-up` command. It walks the same edge from
  the same anchor row as `x`, so `x x X` leaves the line the walk began on and
  a further `X` takes the line above. This deviates from Helix, whose `X` is
  `extend_to_line_bounds`; recorded in `context/reference/helix-keymap-v1.md`.
- A transient `App::line_select` holds the mode that the line selection
  interrupted. Any command other than `x`/`X` ends the selection and hands
  that mode back, so `j` after `x` is a plain motion while `v x j` still
  extends.

Covered by the `line_selection_*` and `counted_line_selection_*` tests in
`src/app.rs`.

## Report

Pressing `x` on a line with text selected the line, and pressing it again
extended the selection to the line below. Both were correct.

On an empty line the first `x` did nothing, which was correct for an empty
line, but the second `x` also did nothing; it should have extended to the line
below.

After the first `x`, pressing `j` or `k` behaved as though `v` had been
pressed first, extending the selection. `j` and `k` should instead leave
selection mode and move the cursor by one line.

Selecting lines upward should be possible with `X`, behaving symmetrically to
`x`. Pressing `x` twice and then `X` once should leave one line selected,
having moved the selection upward; a second `X` should select two lines, the
original plus the one above.
