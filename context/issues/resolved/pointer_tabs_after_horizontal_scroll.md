---
title: "Pointer coordinates disagree with tab rendering after horizontal scrolling"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 984a06b
---

## Resolution

Commit `984a06b` (`fix(input): map scrolled tabs to their displayed cells`).

`App::pointer_offset` used the wrapped-segment cell mapper for every row.
Hard-scrolled rows render tab stops relative to the visible start, so their
pointer mapping now uses `column_for_scrolled_cell`. Wrapped segments keep
`column_for_cell_from`, and table handling remains separate.

Coverage: `left_click_and_drag_use_viewport_relative_tabs_after_horizontal_scroll`
in `src/app/tests/pointer_scrolled_tabs.rs` checks an actual click inside a
scrolled tab and a drag to its following character. It failed with offset 4
instead of 2 before the change and passes afterward.

## Report

After horizontal scrolling, a left click or selection drag over a tab can
select the character after it. Hard-scrolled rows render tab stops from the
viewport's left edge, but `App::pointer_offset` interprets pointer cells using
tab stops from the hidden prefix's original display column.

With soft wrap disabled and tab width 4, open `ab\tz-more-text`, horizontally
scroll by two characters so the tab begins the visible row, and click its
fourth cell. All four cells represent the tab at character offset 2; the
existing mapping selects `-` at offset 4. Dragging from that tab to the
following character also produces the wrong selection endpoints. Right-click
selection hit testing already uses the correct viewport-relative mapping.

Clicks and drags must invert the same tab geometry that renders the row.
Soft-wrapped segments must retain their existing absolute tab-stop mapping,
and rendered table cells retain their separate layout mapping.
