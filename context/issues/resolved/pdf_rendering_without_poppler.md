---
title: "PDF viewing in the native window depends on separately installed Poppler"
status: resolved
reported: 2026-10-08
resolved: 2026-10-09
commit: 1db9279
---

## Resolution

`1db9279` — `Render native PDFs with a bounded Hayro helper and Poppler fallback`
replaces the unconditional Poppler branch of `native_frontend::media::load`.
Previously every PDF request required `pdfinfo`, a PNG-producing `pdftoppm`,
and optional `pdftotext`, so a stock installation could not display PDFs.

`native_frontend::pdf` now invokes a hidden mode of the same executable before
frontend initialization. Exact-pinned Hayro 0.8 crates remain native-only.
The helper owns the parser and caches for one request, returns bounded RGBA and
word metadata over a pipe, and can be killed on cancellation or a 15-second
deadline. It also applies CPU/address-space limits before parsing; macOS's
large initial mappings are measured before adding the 1 GiB budget. The parent
keeps the process-group leader unreaped through cleanup using the repository's
owned-group API. Neither UI loop nor the media worker runs an uninterruptible
Hayro interpreter. Detail rendering translates/scales into only the visible
crop, rather than allocating the potentially 524288px virtual page.

Fallback is automatic per page/refinement request. WarningSink, unresolved
CMaps, fallback font requests and helper-local WARN/ERROR capture detect known
unsupported or damaged content; the log hook is necessary because upstream
font failures do not all reach WarningSink. Standard and embedded fonts work
without system-font lookup; recognized nonembedded Latin names may use built-in
standard substitutes. Empty-user-password encrypted documents work; documents
requiring a password report a clear error.

The text Device combines page/draw/glyph transforms, groups glyphs in content
order by baselines and gaps, retains invisible OCR text, and deduplicates
fill/stroke dispatch. Missing Unicode or Type3 geometry triggers independent
Poppler text extraction. Failed text extraction leaves region selection usable.
Both raster backends and Poppler text use CropBox. Poppler's rotated word boxes
carry unrotated page dimensions, so normalization uses the displayed raster's
aspect to choose the correct axes. This prevents mixed-backend text and detail
misalignment on cropped, rotated pages.

The performance register records release-build comparisons with ten authored
fixtures, including image differences and differing column reading order.
The user guide, backend reference and third-party notices describe fallback,
font substitutions, limits and retained permissive asset notices.

Regression coverage:

- `src/native_frontend/tests/pdf.rs`:
  `hayro_renders_the_authored_fixture_and_extracts_real_words`,
  `hayro_word_geometry_tracks_rotation_and_nonzero_crop_origins`,
  `hayro_detail_is_a_crop_of_the_same_scaled_page`,
  `text_collection_keeps_ocr_and_deduplicates_fill_stroke_and_column_boundaries`,
  `hayro_rejects_missing_fonts_bad_pages_and_damaged_documents`,
  `recognized_nonembedded_latin_fonts_use_documented_standard_substitutes`,
  `private_pdf_response_rejects_truncation_overlarge_headers_and_bad_geometry`,
  and `helper_transport_bounds_output_timeout_cancellation_and_descendant_pipes`.
- `src/native_frontend/tests/media.rs`:
  `mixed_pdf_backends_align_cropped_rotated_text_and_detail`,
  `pdf_rasterizes_distinct_pages_and_reports_total_count`, and
  `pdf_detail_rerenders_vectors_and_matches_full_resolution_crop` (explicitly
  run with `--ignored` and installed Poppler).
- `tests/native_pdf.py`: actual-helper acceptance with Poppler absent from
  PATH, including four crop rotations, columns, vectors, image scans, Form
  XObjects, empty/required passwords, unknown fonts, damaged files and large
  virtual-page crops; `--compare` adds Poppler image/text/timing comparisons.
  Native CI runs the helper acceptance on Linux and macOS.
- `tests/native_window.py`: real-window PDF paging, refinement, text selection
  and clipboard acceptance. Local Linux validation passed alongside formatting,
  default/native Clippy, the full Rust suite, 77 native tests and all three
  explicit Poppler tests. Astra high review iterated to a clean final result.

Known limitation: reading order remains heuristic and follows content-stream
order, and standard font substitutions can alter appearance. Automatic fallback
cannot detect every silent visual difference. The authored corpus does not
establish broad accuracy for CJK, JBIG2/CCITT/JPEG 2000 scans, complex forms,
large papers or slide decks. There is no PDF password prompt, OCR or persistent
parsed-document cache. macOS helper validation is wired into CI but was not
executed locally; performance numbers are Linux-only.

## Report

In `runyte --window`, PDF projections are produced by three Poppler utilities,
invoked through argument vectors from the media worker in
`src/native_frontend/media.rs`:

- `pdfinfo` for the page count (validated to 1–10,000 pages);
- `pdftoppm -singlefile -png` for page rasters, either `-scale-to 1600` for the
  base page or `-scale-to-x`/`-scale-to-y` with `-x`/`-y`/`-W`/`-H` for a
  zoom-refinement detail region, written to a temporary directory and decoded
  again;
- `pdftotext -bbox-layout` for word boxes and reading order, parsed from
  bounded XHTML with `roxmltree`.

Each helper is killed and reaped on cancellation, on exceeding its output-size
limit, or after 15 seconds (`wait` in the same file). `THIRD_PARTY_NOTICES.md`
records that Poppler is neither linked nor bundled.

Consequences of the current design:

- PDF support needs a separate install (`poppler-utils` or `poppler`); stock
  macOS and Windows do not provide it. When it is missing, opening a PDF fails
  with "start PDF renderer (install poppler-utils / Poppler)".
- Behavior depends on the installed Poppler version, including the
  `-bbox-layout` output format the text-selection path parses.
- Every page costs one process spawn plus a PNG encode and decode.
- Poppler is GPL-licensed. Invoking it as a separate program does not affect
  Runyte's MPL-2.0 source, but a distribution that bundled the helpers with
  Runyte would ship GPL binaries alongside it.

Expected: the native window renders PDFs in process with
[hayro](https://github.com/LaurenzV/hayro) (pure Rust, Apache-2.0 OR MIT) by
default, and falls back to the existing Poppler path when hayro cannot load a
document or render a page and the Poppler utilities are available. With neither
working, the pane reports the hayro failure and the missing-Poppler hint.

Scope of the hayro backend, mapped to the three current helpers:

- Page count: `hayro_syntax::Pdf::new(bytes)?.pages().len()`, with the same
  1–10,000 page validation. Encrypted documents load through hayro-syntax's
  RC4/AES support; a password-protected document that needs a user password
  gives a clear pane error, as now.
- Rasters: `hayro::render` for the base page at 1600 pixels on the longest
  axis; `hayro::render_into` with a translated and scaled transform for detail
  regions, so that only the requested region (at most 4096 pixels per axis) is
  allocated even when the virtual full size reaches the current 524,288-pixel
  bound. `vello_cpu` contexts are sized in `u16`, so no context may be created
  at the virtual full size. Output stays RGBA in memory; no PNG or temporary
  file is involved.
- Word boxes: a text-collecting `hayro_interpret::Device` that implements only
  `draw_glyph_run` and records each glyph's `transform()` and `as_unicode()`.
  Grouping glyphs into words and lines and choosing a reading order is Runyte
  code; hayro performs no layout analysis. The result feeds the same
  word-box structures the `pdftotext` parser produces today, so selection,
  copying and keyboard text selection are unchanged. Pages with no
  extractable glyphs keep falling back to region selection.

Constraints:

- hayro has no cancellation, deadline, operation budget or memory cap of its
  own. A hostile or pathological document can run for a long time or allocate
  heavily on a thread that cannot be stopped. The existing guarantees — kill on
  cancel, 15-second limit, bounded output, decoded-allocation limit — must be
  kept. The expected shape is a hidden helper mode of the `runyte` executable,
  driven by the existing spawn/kill/reap code in `wait`, rather than calling
  hayro on the media worker thread. Any other shape must show the same
  bounds.
- hayro's `RenderCache` and parsed `Pdf` use `Rc`/`RefCell` and are not
  `Send`; reuse across pages of one document stays within the process or
  thread that created them.
- hayro 0.8 declares `rust-version = 1.92`; `Cargo.toml` promises 1.88 and the
  `MSRV (Rust 1.88)` CI job runs `cargo +1.88 check --all-targets --locked`
  with default features. hayro must stay an optional dependency enabled only by
  the `native` feature, or the promised MSRV must be raised deliberately.
- Pin hayro crates exactly: the project is pre-1.0 and minor versions break
  its API.
- `embed-fonts` adds about 240 KB of permissively licensed substitutes for the
  14 standard fonts. Non-embedded, non-standard fonts (including CJK) go
  through `InterpreterSettings::font_resolver`; without a resolver, such text
  does not render. Decide whether the resolver consults system fonts or whether
  this is a documented limitation of the hayro path.
- hayro's README states that no performance work has been done yet. Startup
  and per-page cost should be measured against the Poppler path before the
  default changes; see `context/reference/startup-performance.md`.
- The Poppler fallback, its limits, and its install documentation stay intact.
  `README.md`, `docs/user-guide.md` (the PDF support and text-selection
  paragraphs), `context/reference/native-window-experiment.md` and
  `THIRD_PARTY_NOTICES.md` must describe which backend is used and when Poppler
  is still needed.

Questions left undecided in the original report:

- Whether the fallback is automatic per document, per page, or a setting the
  user selects.
- Whether text extraction falls back to `pdftotext` independently of
  rendering when hayro renders a page but its word grouping is judged
  inadequate, and how that would be judged.
- How rendering accuracy is compared before switching the default. A suggested
  check is to render a varied set of documents (papers, multi-column layouts,
  scans with JBIG2/CCITT/JPEG 2000, forms, CJK text, slide decks, damaged
  files) through both backends at the same scale, and compare images, timing
  and extracted text. Third-party test documents must not be committed unless
  their licenses allow it; the existing fixture in
  `src/native_frontend/tests/fixtures/` is Runyte-authored.

Reproduction:

1. Build with `--features native` on a system without Poppler installed.
2. Run `runyte --window` and open any PDF.
3. The pane shows the renderer start failure instead of the page.
