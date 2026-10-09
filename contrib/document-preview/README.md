# Native document preview prototype

Build both executables from this checkout (a current stable Rust toolchain and
Runyte's native platform prerequisites are required):

```sh
cargo build --locked --features native
cargo build --locked --release --manifest-path contrib/document-preview/Cargo.toml
export RUNYTE_PREVIEW_HELPER="$PWD/contrib/document-preview/target/release/runyte-preview-helper"
./target/debug/runyte --window --editor contrib/document-preview/fixtures/markdown.md
```

The helper can alternatively be installed beside the Runyte executable as
`runyte-preview-helper`. It has its own locked dependency graph; default and
native Runyte builds do not link Blitz. No browser installation is needed.
The optimized helper is recommended for interactive scrolling. A debug helper
works too, but CPU rasterization is considerably slower. The native worker retains
the current document, fonts and renderer across scroll/selection requests. It
releases the helper when all previews close or after 30 seconds without a request;
changing the pane/capture replaces its cache. Resizing and zooming relayout the
same document. Ordinary scrolling does not parse or relayout it.

Type `:preview`. With a bare caret it captures the entire current buffer,
including unsaved edits. With selected text it captures only nonempty ranges,
in document order, joining disjoint ranges with one newline. The 128 KiB limit
applies to the capture, so a selected section of a larger file works. The source
text, selection, undo history, and scroll position are retained. Capture is
manual: invoke `:preview` again to refresh the same pane. Escape returns to
source without changing its selection. An explicit SVG selection is detected
by its XML root, including when selected inside Markdown or a source file.

- Wheel scrolls continuously; Shift-wheel scrolls horizontally. Prose wraps
  with pane width. Code, JSON, YAML and plain text preserve whitespace and
  overflow horizontally at document level.
- Built-in PDF/media navigation comes from the existing keymap registry:
  `j`/`k` or arrows scroll, `h`/`l` pan horizontally, `Ctrl-f`/`Ctrl-b` or
  PageDown/PageUp move a viewport, `Ctrl-d`/`Ctrl-u` move half a viewport,
  and `Ctrl-n`/`Ctrl-p` also page. `gg`/`ge` reach the top/bottom.
  `+`/`=` and `-` zoom; `zf`/`z1` restore the normal reflowing view; `zh`/`zl`
  pan. Prefix hints use that same registry. `q` or Escape returns to source.
  Configured editor remappings and counted motions are not supported here yet.
- Left-drag selects document text; `y` or Ctrl-Shift-c copies it (Cmd-c on macOS is
  implemented but not platform-tested). Clicking a link displays its destination;
  navigation is disabled. SVG text is graphical and is not selectable.
- `Space`, `Ctrl-w` and `:` retain editor command routing. Configured Ctrl-arrow
  shortcuts also reach the editor without dismissing the preview, so pane focus
  can move away and back while preserving the document view. Prompts and key hints
  draw above the preview and own input. Escape first dismisses those surfaces,
  then returns to source. Other editing keys return to source before dispatch.
- Use the editor's pane-focus commands before selecting in an inactive preview.
  Clicking an inactive preview does not reposition its source caret.
- A new split shows the source; `:preview` opts that pane in independently.
  Closing or switching away releases the frontend view. At most eight captures
  remain in the host; returning to the same source in the same pane can reopen
  that captured version unless it was dismissed (the frontend remembers eight
  dismissals). A new native attachment can reopen the capture with fresh scroll
  and selection. The terminal frontend shows the source and explains that `:preview`
  requires `--window`. Captures are host memory, not saved persistent state.

`?`, `:render`, and `:markdown` retain their existing generated-text behavior.
They do not invoke Blitz.

## Formats and fixtures

| Fixture | Presentation |
| --- | --- |
| `fixtures/markdown.md` | GFM headings, emphasis, lists, tasks, quotes, tables, highlighted fences, links, local PNG |
| `fixtures/document.html` | Static HTML with embedded CSS, flex layout and media-query reflow |
| `fixtures/data.json` | Pretty-printed highlighted JSON |
| `fixtures/broken.json` | Original JSON text plus parse diagnostic |
| `fixtures/config.yaml` | Highlighted authored YAML, comments, anchors and ordering retained |
| `fixtures/source.rs` | Highlighted source with literal markup in strings |
| `fixtures/plain.txt` | Monospace text with literal markup and indentation |
| `fixtures/drawing.svg` | Inline SVG shapes, gradient, paths and text |

Language detection uses Runyte's existing document language, including explicit
buffer-language metadata and provider syntax hints. Unknown languages fall back
to monospace. Highlighting uses Syntect's bundled grammar set in the helper;
its coverage differs from Runyte's Tree-sitter set. JSON serialization is for
preview only and can reorder object keys. YAML is never serialized. Raw HTML
inside Markdown is escaped. Only HTML documents and explicitly detected SVG
fragments become active document markup.

`preview.css` is a bundled, Runyte-authored MPL-2.0 stylesheet inspired by GitHub's
reading layout. It is not GitHub's stylesheet and does not provide exact GitHub
rendering equivalence. Proportional and code fonts currently use system fonts;
no additional font is bundled by the helper.

## Supported static subset and resource policy

This prototype exercises block/inline flow, headings, lists, tables, borders,
backgrounds, basic typography, flexbox, media queries, raster images and inline
SVG through Blitz/Stylo/Taffy/Parley and Vello CPU. Embedded `<style>` and inline
`style` attributes are supported to the extent implemented by the pinned engine.
Grid, complex CSS, filters, pagination and broad website compatibility are not
acceptance claims. HTML is parsed with HTML5 error recovery. SVG uses Blitz's
usvg integration; shapes/text are rasterized. SVG family attributes retain their
authored families with concrete Noto/DejaVu/Liberation/Arial/Helvetica fallbacks. External SVG images/references,
including SVG `<image>` and `<feImage>`, are removed before layout.

There is no JavaScript engine, website navigation, browser profile, cookies,
download path, form interaction, external stylesheet or web-font fetching.
Animations are sampled at time zero, with no animation timer.

Only PNG/JPEG/WebP files under the canonical directory containing the source
file may be loaded by document references. Relative references resolve there;
absolute file URLs are admitted only within the same directory tree. Symlinks
resolving outside it are refused. A scratch buffer has no asset authority.
HTTP(S), data URLs, CSS imports, linked stylesheets, SVG image files and fonts
are denied. This includes resources requested by inline CSS. Local SVG diagrams
can be opened as text and previewed directly. The file checks are not a sandbox
against concurrent hostile filesystem replacement.

Budgets: 128 KiB captured text, 256 selected ranges, eight host captures, at most
16 local image requests of 2 MiB and 2048×2048 pixels each, 4096 pixels per visible viewport
axis and four million visible viewport pixels (large/HiDPI panes reduce raster resolution
while preserving logical layout and hit coordinates), five seconds wall time per request,
and 2 GiB address space on Linux. The request deadline replaces the former
process-lifetime CPU limit, which cannot apply to a retained renderer. Pending requests are bounded
and coalesced per pane. In-flight scroll frames may finish and be displayed while
a newer target is pending; hidden, refreshed and closed views cancel their work. Engine failures show a pane diagnostic; Escape remains
available. Images, parsing, highlighting, layout and rasterization run off both
editor input loops. No helper or timer starts until a preview is requested.
The process limit is failure containment, not a general OS security sandbox.

Scroll offsets preserve fractional pixels. The active preview keeps an adjacent
raster around the visible area: up to a viewport above/below and a quarter-width
to either side, reduced to fit an eight-million-pixel / 8192-axis cache budget.
This preserves the visible viewport's resolution and CSS layout. Cached scrolling
only moves the image; it does not ask the helper to redraw. Approaching a cache
edge refreshes the surrounding area while it is still painted. A jump beyond the
cache temporarily retains the last fully painted area until the new one arrives,
instead of exposing blank edges. Selection overlays retain the unselected cache.
Resize, zoom, source refresh and close invalidate or release the relevant cache.

The cache is bounded to one unselected raster per pane (at most 32 MB of RGBA)
plus its current viewport/selection raster, shared when they are the same image.
Fixed/sticky positioned HTML disables adjacent caching and uses exact viewport
redraws. Complex viewport-dependent CSS is not a compatibility claim. No wheel
inertia or keyboard-scroll animation is added; precise trackpad deltas survive.

## Verification

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo clippy --features native --all-targets -- -D warnings
cargo test --features native --bin runyte native_frontend
cargo llvm-cov --locked --workspace
cargo fmt --manifest-path contrib/document-preview/Cargo.toml --check
cargo clippy --manifest-path contrib/document-preview/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path contrib/document-preview/Cargo.toml
python3 contrib/document-preview/check.py
# Dedicated X11 display only: the test owns its clipboard.
DISPLAY=:95 python3 tests/native_window.py --document-preview --output /tmp/runyte-preview-captures
```

`check.py` uses `RUNYTE_PREVIEW_HELPER`, falling back to the debug helper and tests actual Blitz selection, links,
scrolling, 2× rendering, all formats, local asset denial and selected SVG dispatch.
The GUI harness uses temporary workspace/configuration/runtime directories.
The [development record](../../context/reviews/native_document_preview.md) states
which checks and platforms were actually verified and the recommendation.
