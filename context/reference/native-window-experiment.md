# Native window experiment

This branch adds an opt-in GPUI 0.2.2 frontend, built with `--features native`
and selected with `--window`. It supports standalone editor and IDE modes.
The experiment stays on `exp`; do not merge this branch into `dev` or `main`.
The terminal frontend remains the default. The public plugin contract and
persistent-session transport do not gain a new client protocol.

## Ownership

GPUI owns the main thread, window, font rendering, image textures, media
viewport transforms, and native clipboard ownership. The ordinary standalone
host loop runs on one worker thread and retains editor commands, configured
keymaps, filesystem operations, language services, Git, and PTYs. The existing
Ratatui renderer writes into a `TestBackend`; GPUI draws its cells and cursor.
No Zed editor widgets or separate editor command implementation are included.

The native desktop identity is `com.runyte.Runyte`. Its icons derive from
`logo/runyte_logo.svg`; the authored charcoal mark sits on a light rounded
backplate. Linux Wayland resolves the matching installed desktop entry. X11
also receives `_NET_WM_ICON` from one bounded background scan matching this
process's PID and exact window class. GPUI 0.2.2's X11 raw-handle accessors are
unimplemented, so the adapter does not call them or vendor the toolkit.
The macOS helper builds a local `.app` with an ICNS resource, launcher and
project/dependency/font notices. Installation, regeneration and platform
limits are documented in [the native packaging guide](../../contrib/native/README.md).

Native builds embed four unmodified JetBrainsMono Nerd Font faces from Nerd
Fonts v3.4.0 (JetBrains Mono 2.304): Medium, Medium Italic, Bold, and Bold Italic.
They are registered before the window opens; both the cell grid and media
labels use them. Font binaries add approximately 9.5 MiB only to native builds.
Licenses and attribution live under `licenses/jetbrains-mono/`.

The bridge retains the latest owned frame and coalesces wakeups. It does not
poll on a GUI timer or enqueue every rendered frame. Physical input carries
the identity of the frame painted when it was acquired. Pointer targeting and
context review use that identity. Native approvals require a painted current
surface; ordinary form text and navigation remain queueable. Plugin action
preflight uses revision witnesses from a bounded history of actual painted
frames, including actions invoked through configured bindings, menus and
colon commands. Paint acknowledgements update viewport observations without
synthesizing editor input or starting redraw loops.

Approval admission currently compares the complete painted frame with the
latest prepared frame and refuses activation while publication is pending.
Unrelated status changes or sustained terminal output can therefore reject
an approval key even when the decision itself has not changed; retrying after
the display catches up is required. This conservative behavior does not drop
ordinary form typing or navigation. Replacing it requires a stable approval
lifecycle witness that includes hidden captured state, not only matching
overlay labels: filesystem plans retain source fingerprints and uploads,
reloads retain observation generations, and secret fields can change while
their displayed bullets remain identical. The existing confirmation counter
does not cover every approval owner or replacement. A common lifecycle
witness is deferred rather than inferring authorization from visual equality.

Media files are read-only generated projections with a separate `media_path`;
they never acquire an editable file path, LSP document or save target. Existing
file workflows intercept supported formats before external binary opening.
PDF projections have one logical row per page. Escape clears native selection
first, then enters the `[pdf]` page buffer; its ordinary text motions, counts
and search choose a row, and Enter displays that page. Escape from page rows
opens the PDF's directory with the file selected. Images use `[image]` and
skip the page-buffer step. `Space e` goes directly to any media source's
directory. Preview/page-buffer state belongs to the pane and is tied to the
buffer identity; source page and view position survive returning via explorer.
The old page-picker overlay and `g p` binding are removed.

The native frontend renders owned snapshots using the same overlay geometry
as the host-frame renderer. Images remain behind key hints and prompts;
only overlay cells paint above them. Media mouse input is blocked while an
overlay owns input, including clicks on hints that would otherwise land on
hidden page rows. Pane page counts are initialized even when a page buffer
is entered before the first raster has rendered.

The `Media` binding scope adds zoom/fit/copy controls and owns its pixel
selection semantics. Core commands emit bounded requests addressed to the
pane, path and page. Zoom, center, drag and extracted-text/region selection
remain pane-local frontend state. Mouse focus names the media document rather
than translating image pixels into synthetic PDF rows. Hit testing uses the
painted media geometry. Source replacement clears stale selections.

One bounded media worker decodes images and invokes external Poppler helpers
using argument vectors. Rasterization and text extraction stay off both UI
loops. The LRU cache holds eight results. A bounded demand queue takes priority
over speculative loading of two PDF pages on each side of the current page. New
demand cancels an unrelated speculative job; speculative completion never
extends its own prefetch window. The one worker blocks on its channel when
idle. Fingerprinted page counts are retained separately in eight bounded
entries so a reopened buffer can attach to cached rasters. Unchanged counts
and speculative completions do not rewrite page rows or trigger redraws.
Input, decoded allocation, dimensions,
page count, helper duration and helper output have explicit limits. Child
processes are killed and reaped on cancellation or limit failure. Scratch
rasters and extracted XHTML live in temporary directories. JPEG orientation
is applied before display, hit testing or copying.

Poppler's `pdfinfo` and `pdftoppm` provide PDF pages; `pdftotext -bbox-layout`
provides normalized word boxes and reading order. `roxmltree` parses bounded
XHTML. Scanned pages fall back to image-region selection, without OCR.
GPUI publishes text clipboard data on Linux; `arboard` supplies image clipboard
ownership because GPUI 0.2.2 does not publish image MIME data to other Linux
applications. Both are Rust integrations; Poppler is installed separately.

## Validation

Run the ordinary repository checks, then the optional adapter checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo clippy --features native --all-targets -- -D warnings
cargo test --features native --bin runyte native_frontend
cargo test --features native --bin runyte \
  native_frontend::media::tests::pdf_rasterizes_distinct_pages_and_reports_total_count \
  -- --ignored --exact
cargo llvm-cov --locked --workspace
```

`src/app/tests/native_media.rs` covers normal file/explorer opening, read-only
identity, startup positions and reuse, non-regular-file refusal, registry-backed
media controls, page navigation, keyboard selection and explicit mouse targets.
Native adapter tests cover input translation, painted-frame identity, approval
refusal, PDF text coordinates, raster colors/alpha, region copying, EXIF,
source invalidation, transforms and worker cleanup. The host's
`plugin_frontend_presentation` tests cover queued form input, LSP permissions,
all plugin action entry points and bounded presentation witnesses.

`tests/native_window.py` drives a real GPUI window on an isolated X11 display.
It requires Python, libX11, libXtst, Git and Poppler. It creates temporary
workspace/config/runtime directories and uses a shell owned by that fixture.
It checks first paint, modal edit/save, image and PDF opening, PDF page changes,
keyboard and pointer zoom, drag pan, mouse and keyboard PDF text selection,
actual system clipboard text/PNG bytes, hints, resize/splits, integrated PTY
input, unsaved-close refusal and successful close after saving.

```sh
cargo build --features native
# Run only against a dedicated display: clipboard tests take clipboard ownership.
DISPLAY=:94 python3 tests/native_window.py --output /tmp/runyte-window-captures
```

The GUI acceptance has been exercised with Xvfb and Mesa lavapipe on Linux.
This proves the X11 rendering/input path, including actual clipboard transfer;
it does not establish macOS, Wayland, hardware-specific Vulkan, IME or touchpad
pinch acceptance. The user guide records these limits and current media keys.

## Measured acceptance — 2026-10-07

On Linux, the default full suite and default/native all-target Clippy passed.
The native adapter suite passed 13 tests, and the separately invoked Poppler
acceptance passed. The final X11 window run passed every interaction above,
including keyboard PDF selection and external clipboard reads. Canonical
workspace coverage reported 92.01% lines (148,278 of 161,158), above the
unchanged 89% floor. This canonical measure uses the default feature set;
optional GPUI drawing is checked by the native tests and GUI acceptance.

A separate six-second KDE Wayland desktop smoke check received toplevel
configuration and attached a surface without errors or panics, then exited
on SIGTERM. This establishes basic Wayland window startup/presentation only;
Wayland gesture, clipboard and IME acceptance still require a platform run.

## Prior acceptance: fonts and page picker — a0646aa, 2026-10-08

That commit initially used `g p` for a shared PDF page picker at the current
page. The subsequent Esc/page-buffer design above replaces that interaction.
Cursor movement and filtering left the document selection unchanged until Enter;
cancellation kept the original page. Rows captured pane/buffer identity and
acceptance rejected stale destinations. `42gg` selected page 42 directly. Global `g p` and
`g P` paragraph bindings were removed to reserve the spelling consistently;
the paragraph commands remain configurable in Normal and Select modes.

The isolated X11 acceptance passed with Fontconfig's system font directories
disabled (`--no-system-fonts`). It exercises actual page-picker filtering and
cursor acceptance as well as previous image/PDF interactions. It also reads
`WM_CLASS` and `_NET_WM_ICON` from the real window and checks dimensions and
ARGB colors. GPUI omits the final NUL in its X11 class property; the adapter
accepts that exact form as well as the standard terminated form.

Default and native all-target Clippy, the full default test suite, 15 native
adapter tests, and three desktop packaging tests passed. Font metadata was
checked for all four requested faces; `cargo package --list` includes the
font binaries, icon assets, source SVG and license notices. The exp-only
`native-window.yml` workflow adds Linux/macOS builds, tests and packaging
checks plus Linux GUI acceptance; it has not run remotely for this change.
macOS Finder/Dock and native interactions still need a Mac validation run.

Canonical `cargo llvm-cov --locked --workspace` verification reported 92.01%
line coverage (148,340 of 161,215), above the unchanged 89% floor. The optimized
release build also passed the full isolated X11 acceptance with system font
directories disabled and the real icon property checked.

## Esc navigation and neighboring-page cache — 2026-10-08

The full default suite, default/native all-target Clippy, formatting checks,
and native adapter tests passed on Linux. Canonical workspace coverage measured
92.05% lines (148,594 of 161,429), above the unchanged 89% floor.

The isolated X11 release acceptance passed with system font directories disabled.
It covers selection clearing before leaving previews, PDF page-buffer motions
and Enter, image/PDF source-directory navigation, restoring a PDF page after
reopening from explorer, and visible media beneath hints. Clicks on the media
and hint rectangles cannot move hidden page rows. A warmed next-page transition
passed its 150 ms display check; this is fixture acceptance, not a latency
guarantee for arbitrary documents. Scheduler tests cover bounded neighboring
prefetch, demand priority and cancellation, promotion of an in-flight neighbor,
LRU eviction, and source invalidation. macOS interaction acceptance remains
unverified locally.
