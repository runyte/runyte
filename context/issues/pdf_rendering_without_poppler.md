# PDF viewing in the native window depends on separately installed Poppler

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

Open questions:

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
