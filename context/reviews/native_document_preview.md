# Native document preview feasibility

Investigated on 2026-10-09 from Runyte `6c478af`, on branch `exp-html`.
This is a static document experiment, not approval or implementation of the
[broader browser-pane proposal](../plans/proposed/PLAN_BROWSER_PANES.md).
The [prototype guide](../../contrib/document-preview/README.md) contains exact
build/run instructions, fixtures, controls, resource policy and supported subset.

## Implementation and ownership

`:preview` captures the current unsaved buffer, or its explicit nonempty selected
ranges. It uses the editor's operative span convention, including inclusive
keyboard selections and half-open pointer selections. A bare caret captures the
whole buffer. Disjoint selections are joined with a newline. A selected SVG root
is recognized even inside Markdown or source code. Capturing and formatting never
change source text, selection, undo history or viewport. Existing `?`, `:render`
and `:markdown` behavior remains separate.

Shared state holds bounded, presentation-neutral text captures keyed by pane;
private bundled-client protocol version 76 carries those values. Native views
own scroll, hit coordinates, selection and images. Blitz types exist only in the
separate, locked `contrib/document-preview` helper crate. The root dependency graph
is unchanged. A split initially shows source; each pane opts in independently.
Switching source or closing drops the view and cancels work. Reattachment can
reopen the host capture with fresh presentation state. Explicit dismissals are
remembered within a bounded frontend cache. Terminal clients retain source and
report that the command requires the native window.

Formatting uses pulldown-cmark GFM, Syntect and an original bundled MPL-2.0
GitHub-like stylesheet. JSON is pretty printed only in the preview; invalid JSON
keeps its original text and a diagnostic. YAML is highlighted without serializing
it. Non-HTML source is escaped. HTML uses static HTML5 parsing; standalone SVG is
validated and malformed SVG falls back to escaped source with a diagnostic.

The CPU renderer produces a clipped viewport image for GPUI. Engine text geometry
supplies selection and hit testing, so this is more than a screenshot viewer.
Editor cell composition masks the source underneath the image while preserving
prompt/hint layers above it. Overlays retain input ownership. Escape returns to
source; other editing keys dismiss preview before normal dispatch. Preview copy
uses Ctrl-Shift-c (Cmd-c is implemented but untested on macOS).

The helper is created on demand, never during ordinary editing. Parsing,
highlighting, fonts, assets, layout and painting run outside both editor loops.
Work queues and responses are bounded; superseded, dismissed and dropped views
cancel their helper. Limits include 128 KiB of captured text, eight captures,
five seconds per-request wall time and Linux address-space containment.
Failures remain dismissible diagnostics. This is not an OS security sandbox.

## Engine evidence and integration boundaries

Blitz is pinned to
[`74fe1abf090732524c86d96e6cdb6f95d8f8d6dc`](https://github.com/DioxusLabs/blitz/tree/74fe1abf090732524c86d96e6cdb6f95d8f8d6dc).
Its [upstream overview](https://github.com/DioxusLabs/blitz/blob/74fe1abf090732524c86d96e6cdb6f95d8f8d6dc/README.md)
describes the Stylo/Taffy/Parley stack and dual MIT/Apache-2.0 licensing;
Stylo carries MPL-2.0. Dependencies and notices are retained separately in the
helper lockfile and repository third-party notices.

The investigation used the pinned source for `HtmlDocument::from_html`,
`DocumentConfig`, `Viewport`, `resolve`, `hit`, `find_text_position`,
`set_text_selection`, `get_selected_text`, `scroll_viewport_by` and CPU painting.
Relevant primary code is in
[blitz-dom](https://github.com/DioxusLabs/blitz/tree/74fe1abf090732524c86d96e6cdb6f95d8f8d6dc/packages/blitz-dom),
[blitz-html](https://github.com/DioxusLabs/blitz/tree/74fe1abf090732524c86d96e6cdb6f95d8f8d6dc/packages/blitz-html), and
[blitz-paint](https://github.com/DioxusLabs/blitz/tree/74fe1abf090732524c86d96e6cdb6f95d8f8d6dc/packages/blitz-paint).

The supported and exercised CSS subset includes block/inline flow, typography,
lists, tables, borders, backgrounds, flexbox and media queries. Local raster
images and inline SVG work. This is neither full browser compatibility nor exact
GitHub formatting. External stylesheets/fonts, network requests, navigation,
scripts and form interaction are deliberately absent.

Several boundaries required explicit handling:

- Blitz viewport scrolling takes the opposite delta sign from stored document
  offsets; selection coordinates must account for physical display scaling.
- Relative references need a hierarchical base URL even when denied. Scratch
  buffers use an inert HTTPS base with no asset authority.
- SVG's image resolver can bypass the normal resource provider. External SVG
  references and all SVG image/filter-image nodes are removed before layout.
- Generic SVG font families did not draw labels in the tested font environment.
  Concrete fallbacks appended to SVG family attributes restored the text.
- Only canonical local PNG/JPEG/WebP descendants of the source directory are
  admitted, with request, byte and dimension limits. Traversal and symlinks out
  are denied. Concurrent hostile filesystem replacement is outside this policy.

## Observed verification

Tested on Linux, X11 through an isolated Xvfb display, using Vulkan lavapipe for
GPUI and Vello CPU for documents. No macOS or native Wayland runtime was tested.
Headless engine checks exercised 2× rendering and logical selection coordinates;
the GUI exercised window resizing and editor font scaling. Physical HiDPI display
behavior is not established by those checks.

The native acceptance harness exercised selection and real clipboard copy, link
hit feedback, scrolling, Markdown reflow, graphical SVG, complete selected-section
capture, overlay composition/input ownership and return to unchanged source. It
also issued editor input during a large document request and verified the saved
source. Fixtures include Markdown tasks/tables/fences/local images, HTML, valid
and invalid JSON, YAML, Rust, plain text and SVG.

Checks completed:

- Root formatting, default and native Clippy with warnings denied, and full
  `cargo test`.
- Native frontend unit tests, including overlay masks and helper cancellation.
- Helper formatting, Clippy, seven formatting behavior tests and actual engine
  acceptance via `contrib/document-preview/check.py`.
- Canonical `cargo llvm-cov --locked --workspace`: **92.04% line coverage**, or
  149,218 of 162,118 lines. The enforced 89% floor was not changed. This measurement
  covers the default workspace; it does not claim helper/native raster coverage.

One debug-build engine acceptance sample measured process startup through output
at 89.5 ms for HTML, 100.7 ms for JSON, 156.4 ms for YAML, 199.9 ms for Rust,
201.6 ms for plain text, 268.6 ms for Markdown and 143.2 ms for SVG. These are small
fixture observations on this machine, not benchmarks or production guarantees.
These initial measurements predate the retained-renderer follow-up below.
There is no recurring preview work before invocation.

## Recommendation and remaining limitations

Retain Blitz as the candidate for static documents; it crossed the difficult
selection, raster composition, scrolling and editor ownership boundaries without
requiring a browser. Do not settle the broader browser engine decision from this
result.

The follow-up below retains a DOM/font context and CPU renderer in a killable
helper and coalesces viewport updates. Next, add overscanned/tiled painting or a
GPU scene path to remove exposed edges during rapid scrolling. Follow that with
packaged fonts, stronger asset isolation and actual macOS,
Wayland and physical HiDPI acceptance before promoting the feature.

SVG text is graphical rather than selectable. Blitz copy joins some block
boundaries with spaces, so copied code may lose authored line breaks. Nested HTML
scroll containers are not separately routed; document-level scrolling is wired.
Complex CSS, SVG filters and full website compatibility are unproven. Markdown
raw HTML is escaped, remote images are denied, and Syntect language coverage
differs from the editor. Preview colors currently use a light document theme.
Large captures are rejected with a useful message; a smaller selection can still
be previewed. Automatic refresh, editing, trees, folding and export are absent.

## Large-window follow-up

A user reported helper failure in a large window. Reproduction at 3030×1650
physical pixels showed the four-million-pixel admission limit rejecting the
request. The frontend now caps raster resolution before submitting it, preserving
logical layout and selection coordinates. Normal-sized panes keep native scale;
large panes trade sharpness for bounded memory. Helper stderr is captured with a
4 KiB retained limit and exit status is shown on failure. Regression coverage
checks large and HiDPI viewport geometry and real reduced-scale engine selection.

## Navigation and continuous scrolling follow-up

The native view now resolves supported navigation through the existing built-in
Media keymap, including its inherited page, half-page and file-edge commands.
Prefix hints use those same entries. Source bindings remain unchanged; configured
remappings and counted motions are not yet forwarded into this local view.

A framed `--serve` helper retains the parsed document, font context, image cache
and CPU renderer. Scroll and selection updates repaint without reparsing or
relayout; resizing/zooming relayout the retained document. The worker caches the
last pane/capture, releases it after 30 seconds idle, and shuts down when no
preview remains visible. Capture refresh replaces the helper so local assets can
refresh too. Per-request wall time replaces the process-lifetime CPU limit.

Pending scroll updates coalesce. In-flight frames may complete and be displayed
without overwriting a newer target. The frontend immediately translates the
current raster using fractional offsets, clamped to engine-reported document
bounds, while the worker fills the new viewport. Hit testing waits for matching
geometry. This avoids freezing the image while painting but can expose a blank
edge on fast motion; overscan/tiles remain a next step. It does not synthesize
inertia or animate keyboard jumps. Real engine tests assert exact 1.25-pixel
scroll increments, selection clearing, zoom/resize and capture replacement.

The optimized helper's retained redraw sample at 1080×800 measured 1.7 ms median
and 2.2 ms maximum for simple HTML (ten fractional scroll updates), and 2.0 ms
median / 2.5 ms maximum for the Markdown fixture (twenty updates). Cold simple
HTML startup was 23.1 ms. Timings include helper IPC/raster output, not GPUI
texture upload, presentation latency or a physical trackpad. The debug helper's
comparable warm simple-page median was approximately 62 ms; the run guide now
recommends the release helper. Native tests cover registry-derived navigation;
the X11 acceptance harness additionally exercises paging, half-pages, `gg`,
zoom/reset and `q`, alongside 3000×1700 window selection. Root tests, formatting,
Clippy and helper checks passed; 82 native tests passed with three ignored.
The earlier default-workspace coverage remains applicable because these
follow-ups change only native-feature code and the separate helper.
