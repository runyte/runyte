Terminal scroll-up, scroll-down, insert-line and delete-line operations shift
the row vectors once per requested line. `Grid::scroll_up`, `scroll_down`,
`insert_lines` and `delete_lines` use repeated `Vec::remove`/`Vec::insert` on
both cells and row identities. On a tall, narrow supported geometry, one
large line count therefore performs quadratic work on the editor thread.

Scrolling or editing a region should move its retained rows once and replace
the vacated rows in linear time. Lines outside the region, cursor behavior,
stable row identities, wrap provenance, background colours, and the bounded
scrollback order must remain intact. Only scroll-up in a top-anchored primary
region should retire rows to history; delete-line must continue discarding
its removed rows.

Reproduce through a one-column, 32768-row emulator by issuing a large `CSI n S`,
`CSI n T`, `CSI n L`, or `CSI n M`. Counts remain clamped to the affected region.
Boundary coverage should include partial regions, zero and excessive counts,
both history policies, repeated scrollback eviction, and wrapped row identity.
