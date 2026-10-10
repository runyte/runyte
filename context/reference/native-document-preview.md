# Native document preview

Current behavior includes the prototype (`04ba89f`), adjacent cache (`9fd18d0`),
pane-focus correction (`6cf4223`) and persistent dismissal fix (`d4fcd70`),
merged into `exp` through `de5c743` on 2026-10-09. The dated evidence below
originated in the investigation from `6c478af` on `exp-html`.
This is a static document experiment, not implementation of the
[broader browser-pane proposal](../plans/proposed/PLAN_BROWSER_PANES.md).
The [prototype guide](../../crates/runyte-preview/README.md) contains exact
build/run instructions, fixtures, controls, resource policy and supported subset.

## Implementation and ownership

`:preview` captures the current unsaved buffer, or its explicit nonempty selected
ranges. It uses the editor's operative span convention, including inclusive
keyboard selections and half-open pointer selections. A bare caret captures the
whole buffer. Disjoint selections are joined with a newline. A selected SVG root
is recognized even inside Markdown or source code. Capturing and formatting never
change source text, selection, undo history or viewport. Existing `?`, `:render`
and `:markdown` behavior remains separate.

Shared state holds bounded, presentation-neutral text captures keyed by pane.
Private bundled-client protocol version 77 carries captures and dismissals.
Native views own scroll, hit coordinates, selection and images. Blitz types exist only in the
workspace library `crates/runyte-preview`, linked only into the desktop edition.
The desktop executable dispatches `--helper preview` before editor startup; the
workspace shares one lockfile. The terminal edition does not link the engine. A split initially shows source; each pane opts in independently.
Switching source or closing drops the frontend view and cancels work. Returning
to the same source/pane or reattaching can reopen an undismissed host capture
with fresh presentation state. Escape, `q` and editing-key exits remove the
matching pane/generation capture from the host, so dismissal survives workspace
switches and reattachment. Captures are host memory, not saved persistent state.
Terminal clients retain source and report that the command requires the native
window.

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
Work queues and responses are bounded; hidden, refreshed and closed views
cancel obsolete work. The retained helper is released when no preview remains
visible or after 30 seconds without a request. Limits include 128 KiB of captured
text, eight captures, five seconds per-request wall time and Linux address-space
containment.
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

## Initial verification — 2026-10-09

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
  acceptance via `crates/runyte-preview/check.py`.
- Canonical `cargo llvm-cov --locked --workspace`: **92.04% line coverage**, or
  149,218 of 162,118 lines. The enforced 89% floor was not changed. This measurement
  covers the default workspace; it does not claim helper/native raster coverage.

One debug-build engine acceptance sample measured process startup through output
at 89.5 ms for HTML, 100.7 ms for JSON, 156.4 ms for YAML, 199.9 ms for Rust,
201.6 ms for plain text, 268.6 ms for Markdown and 143.2 ms for SVG. These are small
fixture observations on this machine, not benchmarks or production guarantees.
These initial measurements predate the retained-renderer follow-up below.
There is no recurring preview work before invocation.

## Remaining limitations

Blitz serves static documents; this integration does not settle the broader
browser-engine decision. The current implementation retains a DOM/font context
and CPU renderer in a killable helper, coalesces viewport updates and caches
adjacent document regions. Large jumps and multiple active previews still incur
raster upload and replacement costs. Packaged fonts, stronger asset isolation
and macOS, Wayland and physical HiDPI acceptance remain future work.

The follow-up sections below preserve the implementation diagnoses and checks
recorded on 2026-10-09; their intermediate behavior and measurements are historical.

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
geometry. This first retained-renderer version could expose a blank edge on
fast motion; the adjacent-cache follow-up below addresses cache-covered scrolling. It does not synthesize
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

## Adjacent-content cache

After committing the prototype as `04ba89f`, `exp` was merged as `c4a75c7` and
pushed to `origin/exp-html`. The merge was clean. Its canonical coverage was
92.04% (149,638 of 162,576 lines), with the 89% floor unchanged.

The separate cache change paints an expanded document-space rectangle around the
visible viewport without changing its CSS dimensions or display resolution.
Normal-sized windows cache roughly one viewport above/below and a quarter-width
on either side; large windows reduce that margin to keep each raster within
eight million pixels and 8192 pixels per axis. The visible viewport retains its
existing four-million-pixel limit. Selection retains the unselected raster.

Scrolling inside the cached guard area only changes GPUI image placement.
Approaching the boundary requests a new surrounding rectangle while the current
viewport is still covered. Late replies cannot rewind a target moved by a cache
hit. Jumps beyond the cached area keep the last fully painted region on screen
until the replacement arrives. Fixed/sticky HTML falls back to exact viewport
painting because those elements cannot safely be translated with document pixels.

The helper test crops an adjacent cached region and compares it byte-for-byte
with a fresh viewport render, with a height media query guarding against accidental
layout changes. Native tests cover prefetch margins, fractional/zoomed coordinates,
large-jump coverage, resize invalidation and stale-result handling. The X11 harness
pauses only its own helper with SIGSTOP and checks forward/reverse cached scrolling
and the bottom edge, then resumes it in a finally block. This proves cached motion
is independent of a redraw, beyond a screenshot or warm-render timing.

The cache is finite; arbitrarily large jumps still wait for the worker. A GPU
scene/tile path could further reduce large-raster upload and replacement costs.
Full website CSS compatibility and other platform/backend support remain unproven.

Cache verification passed on the same Linux/X11 Xvfb + lavapipe backend: the
paused-helper window acceptance, real-engine pixel comparison, nine helper tests,
and 85 native frontend tests (three ignored). Formatting, default/native/helper
Clippy and the full editor test suite passed. No new macOS or Wayland claim is
made. Cached scroll hits perform no renderer request; arbitrary jump latency
remains bounded by rendering a replacement rather than a frame-rate guarantee.

## Pane-focus shortcut correction

Unrecognized preview keys previously dismissed the surface before editor dispatch,
which incorrectly closed it when configured Ctrl-arrow bindings changed pane
focus. Modified directional shortcuts now reach the editor keymap without
changing the preview's visibility, cache, selection or scroll. Single-key focus
commands from the built-in registry follow the same route. Ordinary arrow keys
still navigate the document. Regression coverage includes all four directions
in `src/native_frontend/tests/preview.rs` and configured horizontal/vertical pane
switches with return-to-preview navigation in `tests/native_window.py`.

## Persistent dismissal correction

Dismissal previously existed only in the frontend's bounded tombstone map.
Workspace attachment changes reset that map while the retained host still held
the capture, so a dismissed view reappeared on return. The native frontend now
queues a pane/generation dismissal, wakes the editor loop, and drains the request
before subsequent input. Standalone mode removes the capture directly;
persistent mode sends the private `DismissDocumentPreview` request. Only the
interactive native attachment may use it. Attachment identity prevents delivery
to a different workspace; capture generation rejects delayed requests after a
refresh. The source buffer and its editing state remain untouched.

Private protocol version 77 requires matching client and host binaries. Existing
sessions should be saved and stopped through a compatible client before upgrading;
a window-only restart does not upgrade its retained host. Undismissed captures
can still reopen on reattachment, but `q`, Escape and editing-key exits remove the
host capture. An explicit `:preview` creates a fresh generation normally.

Behavior coverage in `src/app/tests/document_preview.rs` checks retained state,
idempotence, stale generations, missing panes, source preservation and explicit
reopen. `src/workspace/transport_shared.rs` checks wire decoding and interactive
role admission. `tests/native_window.py --document-preview --mux` exercises real
workspace visits, both dismissal keys, an independent second workspace's preview,
and explicit reopen. Runtime fixtures use isolated configuration and storage.

Verification on Linux/X11 with Xvfb and lavapipe passed both the persistent
workspace round-trip above and the standalone preview acceptance suite (including
pane focus, scrolling, clipboard, overlays, resize, display scaling, SVG and
selected sections). Root formatting and default/native Clippy checks passed;
`cargo test` passed 4,695 tests and the native frontend suite passed 86, with
55 and three ignored respectively. Canonical `cargo llvm-cov --locked --workspace`
reported 92.05% line coverage (149,673 / 162,603), above the 89% floor.
No macOS or Wayland runtime was tested.

## Fast pane key follow-up

The pane-focus correction recognized configured Ctrl-arrow chords but resolved
other single keys against the default registry, which excludes the optional
`editor.fast_pane_keys` bindings. Ctrl-h/j/k/l therefore dismissed the preview
before moving focus. The frontend now recognizes these chords with the shared
`is_fast_pane_key` classifier and forwards them without dismissal; the editor
still decides whether they are enabled. Forwarding clears an unfinished preview
navigation prefix. This does not extend preview navigation to arbitrary configured
bindings.

`native_fast_pane_strokes_and_repeats_reach_the_editor` in
`src/native_frontend/tests/preview.rs` covers native key conversion, repeated keys,
and enabled/disabled registry resolution. `tests/native_window.py --document-preview`
enables fast pane keys and checks both horizontal and vertical motion, alongside
the configured Ctrl-arrow checks; horizontal motion checks editing in the target
pane and navigation on returning to the preview.

Validation: formatting, default/native Clippy with warnings denied, and all 87
native frontend tests passed (three ignored). The GUI harness was syntax-checked
but not executed because this environment lacks an X11 test server. Full default
suite attempts at default and four-thread concurrency reported unrelated context
transport/process failures and stalled; sampled failing tests passed individually.
The physical Ctrl-arrow symptom was not independently reproduced; this follow-up
fixes the confirmed omission of the optional Ctrl-h/j/k/l bindings.

## Desktop packaging

Desktop packaging now requires the helper and copies it beside the native
editor, including into `Contents/MacOS` for local app bundles. The crates remain
independent. The tag workflow builds both and publishes a separate Linux x86-64
desktop archive only after engine and window acceptance from an extracted copy.
The window check exercises internal helper launch from the packaged editor.
Terminal artifacts and their curl installer remain unchanged. Native CI builds
and checks the macOS bundle's engine, but macOS desktop publication remains
withheld pending window acceptance. Packaging commands and platform requirements
live in `contrib/packaging/README.md`; release ordering lives in `releasing.md`.

Local packaging validation on Linux passed both optimized builds, five Python
packaging tests, seven release-packaging tests, twenty installer tests, workflow
linting and historical/current checksum assembly checks. The extracted desktop
archive passed the real engine suite and X11/lavapipe window acceptance with
sibling helper discovery, selection/copy, scrolling, pane focus, resize and
return to source. macOS bundle layout is covered by the packaging tests; no
macOS runtime validation was performed locally.

## Shared desktop lock (editions Phase 4)

The separate preview lock was folded into the workspace lock. Blitz remains at
`74fe1abf090732524c86d96e6cdb6f95d8f8d6dc` and Taffy at
`2d936b7701f64b0fe95e50f1bfdcfc65a9f88f6f`. (Full source revisions are in the
manifest.) GPUI and Fontique now both select dynamic Fontconfig loading on
Linux. The local Stylo derive patch supplies explicit formatting error types;
see `vendor/stylo_derive/RUNYTE-PATCH.md`.

The table lists every preview package whose version set differs in the shared
lock. Multiple versions can represent independent GPUI/core and preview branches;
a newly present parallel version does not necessarily change preview's resolved
edge. Packages present only in the root graph are omitted.

| Package | Previous preview lock | Shared workspace lock |
| --- | --- | --- |
| `aho-corasick` | 1.1.5 | 1.1.4 |
| `base64` | 0.23.1 | 0.22.1, 0.23.1 |
| `bitflags` | 2.13.2 | 1.3.2, 2.13.1 |
| `cfg-if` | 1.0.5 | 1.0.4 |
| `darling` | 0.20.11 | 0.20.11, 0.23.0 |
| `darling_core` | 0.20.11 | 0.20.11, 0.23.0 |
| `darling_macro` | 0.20.11 | 0.20.11, 0.23.0 |
| `derive_more` | 2.1.1 | 0.99.20, 2.1.1 |
| `displaydoc` | 0.2.7 | 0.2.5 |
| `either` | 1.19.0 | 1.17.0 |
| `fastrand` | 2.5.0 | 1.9.0, 2.5.0 |
| `fearless_simd` | 0.7.0 | 0.7.0, 1.1.0 |
| `foldhash` | 0.2.0 | 0.1.5, 0.2.0 |
| `fontdb` | 0.24.0 | 0.16.2, 0.23.0, 0.24.0 |
| `futures-core` | 0.3.34 | 0.3.33 |
| `futures-task` | 0.3.34 | 0.3.33 |
| `futures-util` | 0.3.34 | 0.3.33 |
| `hashbrown` | 0.17.1 | 0.14.5, 0.15.5, 0.16.1, 0.17.1 |
| `heck` | 0.5.0 | 0.4.1, 0.5.0 |
| `idna_adapter` | 1.2.2 | 1.2.1 |
| `imagesize` | 0.15.0 | 0.13.0, 0.15.0 |
| `indexmap` | 2.14.2 | 2.14.0 |
| `itertools` | 0.14.0 | 0.13.0, 0.14.0 |
| `js-sys` | 0.3.106 | 0.3.103 |
| `kurbo` | 0.13.1 | 0.11.3, 0.13.1 |
| `libc` | 0.2.190 | 0.2.189 |
| `litemap` | 0.8.3 | 0.8.2 |
| `log` | 0.4.34 | 0.4.33 |
| `objc2` | 0.6.5 | 0.6.4 |
| `phf` | 0.14.0 | 0.13.1, 0.14.0 |
| `phf_generator` | 0.14.0 | 0.13.1, 0.14.0 |
| `phf_macros` | 0.14.0 | 0.13.1, 0.14.0 |
| `phf_shared` | 0.14.0 | 0.13.1, 0.14.0 |
| `png` | 0.18.1 | 0.17.16, 0.18.1 |
| `potential_utf` | 0.1.6 | 0.1.5 |
| `powerfmt` | 0.2.1 | 0.2.0 |
| `quick-xml` | 0.42.0 | 0.41.0, 0.42.0 |
| `read-fonts` | 0.41.0 | 0.41.0, 0.43.3 |
| `redox_syscall` | 0.5.18 | 0.2.16, 0.5.18 |
| `regex-automata` | 0.4.18 | 0.4.16 |
| `rustc-hash` | 2.1.3 | 1.1.0, 2.1.3 |
| `serde_spanned` | 1.1.2 | 0.6.9, 1.1.1 |
| `skrifa` | 0.44.0 | 0.44.0, 0.46.2 |
| `smallvec` | 1.16.2 | 1.15.2 |
| `smol_str` | 0.3.6 | 0.2.2, 0.3.2 |
| `strum` | 0.28.0 | 0.26.3, 0.27.2, 0.28.0 |
| `strum_macros` | 0.28.0 | 0.26.4, 0.27.2, 0.28.0 |
| `svgtypes` | 0.16.1 | 0.15.3, 0.16.1 |
| `syn` | 2.0.119, 3.0.6 | 1.0.109, 2.0.119, 3.0.3 |
| `synstructure` | 0.13.2, 0.14.0 | 0.13.2 |
| `taffy` | 0.14.0 | 0.14.0, 0.9.0 |
| `tendril` | 0.5.1 | 0.4.3, 0.5.1 |
| `thiserror` | 2.0.21 | 1.0.69, 2.0.19 |
| `thiserror-impl` | 2.0.21 | 1.0.69, 2.0.19 |
| `time` | 0.3.55 | 0.3.54 |
| `tiny-skia-path` | 0.12.0 | 0.11.4, 0.12.0 |
| `toml` | 1.1.8+spec-1.1.0 | 0.8.23, 1.1.6+spec-1.1.0 |
| `toml_datetime` | 1.1.2+spec-1.1.0 | 0.6.11, 1.1.1+spec-1.1.0 |
| `toml_parser` | 1.1.5+spec-1.1.0 | 1.1.3+spec-1.1.0 |
| `toml_writer` | 1.1.3+spec-1.1.0 | 1.1.2+spec-1.1.0 |
| `unicode-ident` | 1.0.26 | 1.0.24 |
| `usvg` | 0.48.1 | 0.45.1, 0.48.1 |
| `wasm-bindgen` | 0.2.129 | 0.2.126 |
| `wasm-bindgen-macro` | 0.2.129 | 0.2.126 |
| `wasm-bindgen-macro-support` | 0.2.129 | 0.2.126 |
| `wasm-bindgen-shared` | 0.2.129 | 0.2.126 |
| `windows` | 0.62.2 | 0.56.0, 0.61.3, 0.62.2 |
| `windows-collections` | 0.3.2 | 0.2.0, 0.3.2 |
| `windows-core` | 0.62.2 | 0.56.0, 0.61.2, 0.62.2 |
| `windows-future` | 0.3.2 | 0.2.1, 0.3.2 |
| `windows-implement` | 0.60.2 | 0.56.0, 0.60.2 |
| `windows-interface` | 0.59.3 | 0.56.0, 0.59.3 |
| `windows-link` | 0.2.1 | 0.1.3, 0.2.1 |
| `windows-numerics` | 0.3.1 | 0.2.0, 0.3.1 |
| `windows-result` | 0.4.1 | 0.1.2, 0.3.4, 0.4.1 |
| `windows-strings` | 0.5.1 | 0.3.1, 0.4.2, 0.5.1 |
| `windows-sys` | 0.61.2 | 0.48.0, 0.52.0, 0.59.0, 0.60.2, 0.61.2 |
| `windows-threading` | 0.2.1 | 0.1.0, 0.2.1 |
| `winnow` | 1.0.4 | 0.7.15, 1.0.4 |
| `yoke-derive` | 0.8.4 | 0.8.2 |
| `zerofrom` | 0.1.8 | 0.1.7 |
| `zerofrom-derive` | 0.1.8 | 0.1.7 |
