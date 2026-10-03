Large result lists and completion menus construct presentation text for every
matching item on each frame. `App::overlay_snapshots` clones labels, details,
tints, and fuzzy emphasis before discarding rows outside its 512-row snapshot
window. The standalone list and completion renderers then independently build
Ratatui items for the full match list even though only a small viewport is
visible.

For a list of thousands of long labels, redraw work and temporary allocations
therefore scale with all matching text rather than the retained snapshot and
visible viewport. Scrolling or unrelated redraws repeat this work without
changing the query. The result list should retain its existing filtering,
ordering, sections, selected row, report scrolling, and snapshot metadata while
formatting only rows that can be published or displayed. Matching still needs
to inspect the complete candidate list.

Reproduce with a result list or language-completion response containing several
thousand matching items. Capture overlays and render a small terminal frame
with a selected row near the end. Inspect presentation construction before the
snapshot row limit and before the renderer viewport limit; the entire result
set is formatted despite almost all rows being discarded or remaining offscreen.
