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
