---
title: "Clipped text rows invent line endings and duplicate end carets"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 7cdf1bb
---

## Resolution

Commit `7cdf1bb` (`fix(rendering): show line endings only after visible text`)
tracks the last document character actually drawn while building text runs.
Previously a spare display cell after clipping a wide glyph or tab could be
mistaken for the true end of the line. Both the line-ending marker and fallback
caret now require the preceding text to have been drawn. When the line-ending
marker already represents the caret, the fallback blank is suppressed.
The existing viewport-bounded scan remains intact.

`clipped_rows_do_not_invent_line_endings_or_duplicate_end_carets` in
`src/snapshot.rs` failed before the change and passed after it. It covers wide
glyph and tab clipping and the single caret on a visible newline marker.
The parent reviewer checked the character loop, wrapping branch, marker
conditions and fallback caret behavior independently.

## Report

With whitespace markers enabled, an unwrapped row whose final visible cell
cannot fit the next wide character or tab can show `¬` at that cell even though
the real line ending is offscreen. For example, an eight-cell text viewport
showing `aaaaaaa界\n` renders `aaaaaaa¬`. The snapshot producer checks spare
screen width and whether this is a final wrapped segment, but does not check
that it actually consumed the remaining document characters.

The same end-of-line decoration path can emit two caret cells. With the caret
at the end of `a\n` and whitespace markers enabled, it marks `¬` as the caret,
then appends another caret-styled blank after it.

Only decorate a line ending after its preceding text is visible, and let the
existing line-ending marker own the single caret cell. Preserve bounded scans
of long lines, ordinary clipping, wrapping, and the fallback blank caret when
whitespace markers are disabled or no line terminator exists.
