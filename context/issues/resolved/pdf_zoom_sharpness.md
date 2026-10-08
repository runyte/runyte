---
title: "PDF text and vector graphics lose sharpness when zoomed in"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: 81cd60a
---

## Resolution

Commit `81cd60a` (`Render sharp visible PDF regions at native zoom`) fixes
the fixed-resolution PDF display in `src/native_frontend/media.rs` and
`NativeView::render` in `src/native_frontend.rs`. The loader previously produced
only a 1600-pixel longest-axis page raster, and the frontend enlarged that same
image at every zoom level. Fonts and vector paths therefore lost detail even
though the PDF retained their original geometry.

`Detail::visible` now computes a visible-region render from the pane's current
geometry and the window's physical pixel density. Poppler renders that region
at the requested page scale, with explicit crop bounds; its
`-scale-dimension-before-rotation` option keeps the coordinates correct for
rotated pages. Rendering only the visible region avoids allocating a full-page
raster at high zoom. Each region is bounded to 4096 pixels per axis and roughly
eight million pixels, while the existing background worker, cancellation,
decode limits, and subprocess deadlines remain in force.

The base page remains visible until refinement finishes. Detail cache keys
include source identity, page, resolution, and crop bounds. The separate bounded
detail cache cannot evict the base page used for selections, copying, and hit
testing. At the end of each frame the loader cancels detail work no visible
pane needs, allowing split panes to retain independent zoom and pan positions.
Only the exact requested detail is displayed; a late render cannot replace a
newer view. Refinement failures leave the base page usable and report the error
inside the pane. Both standalone and persistent-session windows use this shared
frontend path. No commands or keybindings change.

Regression coverage:

- `pdf_detail_tracks_zoom_density_and_visible_document_coordinates` and
  `pdf_detail_bounds_extreme_views_and_rejects_empty_or_invalid_geometry` in
  `src/native_frontend/tests/media.rs` cover zoom, display density, margins,
  empty views, coordinate placement, and bounded output.
- `pdf_detail_rerenders_vectors_and_matches_full_resolution_crop` in
  `src/native_frontend/tests/media.rs` uses real Poppler renders to distinguish
  vector re-rendering from image enlargement, compare cropped and full renders
  at all four page rotations, and exercise a bounded crop at 64x zoom. This
  test requires Poppler and is run explicitly with `--ignored`.
- `detail_changes_cancel_obsolete_work_but_preserve_other_visible_panes`,
  `detail_cache_is_bounded_separate_from_base_pages_and_invalidated_with_source`,
  and `detail_failures_keep_base_page_available_and_do_not_retry_each_frame` in
  `src/native_frontend/tests/media_loader.rs` cover cancellation, independent
  panes, stable selection/viewport geometry, source replacement, cache limits,
  and failure fallback.

Known limitation: unusually large or dense displays can exceed the bounded
refinement budget. Scanned pages and embedded images retain their source
resolution limits. Region copying continues to use the base page raster rather
than requesting a separate high-resolution export. During active zooming or
panning the base raster can remain visible until the current refinement loads.

## Report

In Runyte's native PDF viewer, text and vector graphics become blurry or
pixelated when zooming in. Other PDF viewers keep the same content sharp at
higher zoom levels. The report describes a loss of crispness, but does not
identify an earlier Runyte version where zoom remained sharp.

Expected behavior: PDFs containing fonts and vector graphics should render at
a resolution appropriate to the current zoom and display pixel density.
Scanned pages and embedded raster images remain limited by their source
resolution.

To reproduce:

1. Open a PDF containing text and vector graphics with `runyte --window example.pdf`.
2. Zoom in several times with `+` or Ctrl-wheel (Cmd-wheel on macOS).
3. Compare the enlarged text and vector edges with the same page at a similar
   zoom level in another PDF viewer.

The current implementation explains the limitation:
`src/native_frontend/media.rs` invokes `pdftoppm` with `-scale-to 1600`, producing
a page raster with a fixed longest-axis size. `src/native_frontend/viewport.rs`
scales that raster when zoom changes. The user guide documents this bounded
raster magnification. Enlarging it cannot recover the PDF's original vector
detail.

This is fixable by rendering PDF content again at a resolution appropriate to
the view, potentially using visible-region tiles at high zoom to bound memory
and rendering cost. A larger fixed raster alone would only postpone the loss
of sharpness. The rendering strategy remains to be chosen.

Any implementation must preserve background rendering, cancellation and bounded
resource use; account for resolution in cached results; and preserve the page,
pan position, pointer zoom anchor, and text/region selection when replacing a
raster. Rapid zoom changes must not let stale rendering results replace the
current view. Existing standalone and persistent-session PDF views both need
the corrected behavior.
