---
title: "Large overlays format offscreen rows on every redraw"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 106b22f
---

## Resolution

Commit `106b22f` (`perf(pickers): format only retained and visible rows`).

`App::overlay_snapshots` now selects its 512-row result-list/completion window
before cloning labels, details, styles or fuzzy emphasis. The standalone list
and completion renderers likewise format only their actual viewport. Lightweight
candidate filtering and display-row identities still cover the full list;
selection, section headings, report offsets and snapshot totals retain their
existing meanings.

Coverage: `large_list_snapshots_preserve_sections_selection_and_report_offsets`
and `large_completion_snapshot_keeps_sorted_selection_in_the_bounded_window`
in `src/app/tests/search_and_pickers.rs`, plus
`large_list_viewport_preserves_the_selected_row_and_its_following_section`
in `src/ui.rs`. All 120 UI tests and 27 selected large-input tests passed
(three explicit stress tests remain ignored). Independent review found no
remaining issue. This change reduces formatting/allocation work; no elapsed-time
speedup is claimed.

## Report

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
