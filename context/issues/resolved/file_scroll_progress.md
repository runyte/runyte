---
title: "The status line gave the cursor's row but not how far through the file it was"
status: resolved
reported: 2026-08-15
resolved: 2026-08-15
legacy_commit: eeb9af8
---

## Resolution

Implemented in commit `eeb9af8`, "Show progress through the file in the status
line".

`App::snapshot` filled `StatusSnapshot` with the cursor position and nothing
about the buffer it sat in, so `draw_normal_status` could render `412:17` but
had no second number to compare `412` against. The row number therefore only
meant something to someone who already knew the file's length.

`StatusSnapshot` now carries `line_count`, the active buffer's row count, and
`StatusSnapshot::progress_percent` reads it together with `cursor.row` into a
whole percentage. The computation lives on the snapshot rather than in
`draw_normal_status` because the same snapshot travels over the frame protocol
to attached TUIs; a local and an attached frontend drawing one frame cannot
disagree about the number if only one place derives it. `line_count` rather
than a pre-rendered percentage is what crosses the protocol, so a frontend that
wants a different presentation of the same distance still has the input.

The percentage follows the cursor row, not the topmost visible row. The
requested display was position within the file beside the existing row and
column, and those values already follow the cursor; `Z j` and the other
view-scroll commands move `scroll_row` without
moving the cursor, and that is a look elsewhere rather than a move. Rounding is
to nearest, with two ends clamped: only the first row reads `0%` and only the
last reads `100%`, so an interior row a rounding step from either end is
reported as `1%` or `99%` rather than claiming an end the cursor has not
reached. A buffer of one row has no distance to cover and reads `100%`.

The percentage joins the cursor with a middle dot rather than the `│` that
separates the status line's other fields, because it is another reading of the
same position rather than a separate field: `412:17 · 34%`.

`theme_name` was removed from `StatusSnapshot` and from the protocol's mirror
of it, as the report asked. `App::theme_name` itself is unchanged; it is still
what `:theme`, `:config`, and `Space o t` read and write.

Covered by `src/snapshot.rs::tests::progress_runs_from_the_first_row_to_the_last`,
`src/snapshot.rs::tests::only_the_two_ends_of_a_file_read_as_nought_and_a_hundred`,
`src/snapshot.rs::tests::a_buffer_with_no_distance_to_cover_reads_as_complete`,
`src/snapshot.rs::tests::progress_follows_the_cursor_row_and_the_buffer_length`,
`src/snapshot.rs::tests::scrolling_a_pane_away_from_the_cursor_leaves_progress_alone`,
and
`src/ui.rs::tests::the_status_line_carries_progress_beside_the_cursor_and_no_theme_name`.

Known limitation: the percentage counts rows, so it measures distance through
the buffer's lines rather than through its characters or its rendered height.
In a file of very uneven line lengths, or with soft wrap on, the figure will
not match how much of the pane's scrollable height lies above the cursor.

## Report

The status line showed the current line and column for an open text file but
gave no sense of position within the file as a whole — how far from the start
and how far from the end the cursor was.

The suggested form was a percentage of the file scrolled, displayed next to the
row and column count.

The status line was becoming noisy, so the active theme's name should be
removed from it. That information is easily reached through `Space o t`.
