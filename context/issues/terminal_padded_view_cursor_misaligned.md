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
