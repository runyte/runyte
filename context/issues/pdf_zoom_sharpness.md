# PDF text and vector graphics lose sharpness when zoomed in

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
