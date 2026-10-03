---
title: "Terminal view padding separates the cursor from its displayed row"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 962be54
---

## Resolution

Commit `962be54` (`fix(terminal): align cursor coordinates with padded view rows`).

TerminalSession::live_view inserted top padding into cell and line-ID rows but reported the cursor against the unpadded content. The view now adds the same padding offset to its cursor row. Child cursor coordinates, terminal geometry and clipped-view behavior remain unchanged.

Coverage: padded_live_views_keep_the_cursor_with_its_terminal_row in src/terminal/mod.rs verifies padded views with and without history, equal and clipped heights, the unchanged raw cursor, and hidden-cursor behavior. The regression failed before the fix and passes afterward.

## Report

When a terminal view requests more rows than the retained history and live
screen contain, `live_view` adds blank rows at the top of its cell and row-ID
lists. It computes the cursor against the unpadded rows, however, so the
cursor points into the padding rather than at the displayed terminal content.

The cursor in a `TerminalView` must refer to the same displayed row as its
corresponding terminal cell. Top padding must shift cursor coordinates by the
same amount as the rows, without changing the child cursor or terminal size.
Normal-size and clipped views must retain their current cursor positions.

Reproduce by creating a two-row terminal, writing `one` and `two` on separate
rows, then requesting a four-row view. The displayed rows are two empty rows,
`one`, and `two`; the cursor must be on the final displayed row, not on the
second empty row.
