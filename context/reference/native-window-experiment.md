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
as the host-frame renderer. Window margins, media fills and `Reset` cell
colours use the active theme's background and foreground, carried in each
frame with its cells; a `reset` theme colour becomes a light or dark default
that contrasts with the colour the theme defines.

Scrolling outside media panes sends the host one scroll event, which moves
three lines or columns, per wheel notch, as a terminal does. Touchpad and
high-resolution deltas accumulate per axis into whole events, the dominant
axis wins, and the remainder is discarded on reversal, after a 500 ms pause or
when a platform reports a new gesture. Images remain behind key hints and prompts;
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

## Cell painter and latency measurements — 2026-10-08

The native view shares its latest immutable frame with the canvas through `Rc`.
The cell painter retains up to 1,024 symbol layouts per font face (four faces),
with a 256-byte per-symbol cache limit and FIFO eviction. Each symbol is shaped
independently, preserving the existing absence of cross-cell ligatures and
placing fallback glyphs at their cell origin. Foreground colours and decorations
are applied at paint time, so theme changes do not invalidate glyph layouts.
Glyphs use GPUI's public monochrome/emoji paint paths directly; decorations are
submitted first to preserve the previous text layer's ordering over colour emoji.
Ordinary rows share one logical text layer; the cursor row and rows with oversized
or zero-width font advances keep per-cell layers to preserve ordering at overlaps. Backgrounds
merge into horizontal colour runs, including root-coloured cells to preserve
the same ordering floor on adjacent rows. Rows intersecting media retain the
original per-cell backgrounds and layers because media has its own scene depth.
Presentation acknowledgements and
painted-frame admission retain their existing semantics.

The earlier issue attributed batching failure categorically to per-cell layers.
GPUI can batch disjoint layers with the same draw order. This change removes
repeated layout/cache locking, per-cell allocations and layer bookkeeping;
measurements below concern their combined CPU cost.

Set `RUNYTE_NATIVE_PAINT_TIMING=1` to log `native-paint COLSxROWS Nus` to stderr.
This opt-in trace measures cell-painter CPU scene construction, excluding GPU
submission, display refresh and input-to-presentation latency. It records no
buffer or keystroke contents and adds no timer or wakeup. With tracing disabled,
the painter does not read the clock.

The isolated X11 harness has two focused modes in addition to full acceptance:

```sh
# Dedicated X11 display only; use the same release profile and fonts for both binaries.
RUNYTE_NATIVE_PAINT_TIMING=1 DISPLAY=:94 python3 tests/native_window.py \
  --binary /tmp/runyte-before --paint-benchmark --output /tmp/paint-before
RUNYTE_NATIVE_PAINT_TIMING=1 DISPLAY=:94 python3 tests/native_window.py \
  --binary target/release/runyte --paint-benchmark --output /tmp/paint-after
DISPLAY=:94 python3 tests/native_window.py --binary /tmp/runyte-before \
  --paint-styles --output /tmp/styles-before
DISPLAY=:94 python3 tests/native_window.py --binary target/release/runyte \
  --paint-styles --output /tmp/styles-after --paint-reference /tmp/styles-before
```

The benchmark fills a 120×40 window with repeated text, warms the painter, types
104 letters with 25 ms between keys, retains every paint sample in that interval,
and verifies the saved text. Samples are frames, not individual keystrokes:
coalescing and background redraws can change their count. The style comparison
checks fixed terminal rows pixel-for-pixel, including font weights/styles,
reverse/dim/hidden text, decorations, colour emoji, combining symbols, Nerd Font
icons and wide-cell advancement. Actual CJK glyph appearance remains unverified
on a machine without a CJK fallback font; missing-glyph boxes still test advance.

### Measured acceptance

Three alternating before/after release runs on Linux 7.2.8, AMD Ryzen AI 9 365,
Xvfb and Mesa lavapipe, at 1080×800 logical pixels (120×40 cells). Builds and
tests finished before measurement; ordinary desktop applications remained
running. The baseline is `62fe897` with only the
[timing observation patch](../../benchmarks/native-paint-before.patch) applied.
The same harness and fonts were used for both binaries. Every run verified that
all typed letters reached the saved document; no successful paint samples were
removed. The [individual samples and binary hashes](../../benchmarks/results/native-paint-2026-10-08.json)
retain the measurement provenance.

| Round | Before median (min–max), ms | After median (min–max), ms |
| --- | ---: | ---: |
| 1 | 13.560 (12.830–19.241) | 0.989 (0.385–1.446) |
| 2 | 13.494 (12.901–20.664) | 0.933 (0.388–1.542) |
| 3 | 13.765 (12.895–29.118) | 0.797 (0.391–1.440) |

The median of the three run medians fell from 13.560 ms to 0.933 ms, about
14.5 times less CPU paint time for this fixture. This establishes a reduction
in the diagnosed painting bottleneck; it does not establish a platform-wide
input-to-display latency guarantee or a comparison with a terminal emulator.

Formatting, default/native all-target Clippy, the full default test suite,
31 native adapter tests and the explicitly invoked Poppler test passed. Canonical
`cargo llvm-cov --locked --workspace` reported 92.05% line coverage
(148,738 of 161,582), above the unchanged 89% floor. The final release build
passed full isolated X11 acceptance with system fonts disabled and exact
styled-cell pixel comparison against the baseline with system fallback fonts.
The latter includes 20 styled rows, alternating root/nondefault backgrounds,
emoji and non-emoji runs, plus a cursor in a blank cell beside an italic icon.
Actual CJK glyphs and macOS interaction/latency remain unverified locally.

## Window font and clipboard controls

`editor.font_size` selects 8–48 logical pixels (default 15), read before every
window opens, independently of a retained host's configuration. Settings saves
and reloads report it as startup-bound. Ctrl-plus/equal and Ctrl-minus change
only the current window. `CellMetrics` scales the cell painter, pointer and
media geometry, IME caret and terminal dimensions; resizing invalidates shaped
glyph caches. Reserved window controls and their help use `keymap::native_window`.

Native-feature builds retain an `arboard` clipboard connection in the host's
clipboard port. This makes editor clipboard commands independent of Unix
helper executables and preserves X11 ownership after a write. Media text copy
uses GPUI and image copy retains its arboard owner. Ctrl-Shift-c reaches the
registry's copy command; Ctrl-Shift-v reads clipboard text through GPUI and
sends one bounded literal paste, including to terminal children and prompts.
Normal-mode text paste retains the existing modal clipboard commands.

The focused X11 acceptance is `tests/native_window.py --window-controls`,
with `--mux` to include existing-host reattachment and a parent editor wait.
It verifies text copy through both copy bindings, external clipboard reads,
Ctrl-v and Ctrl-Shift-v, terminal paste, PTY size changes and configuration on
reopening. All clipboard ownership is confined to the dedicated test display.

The vendored `proc-macro-error2` 2.0.1 carries only the public `proc_macro`
visibility correction needed by Rust's future-incompatibility check. GPUI
0.2.2 brings it through stacksafe 0.1.4; see `vendor/proc-macro-error2/RUNYTE-PATCH.md`.

### Validation — 2026-10-08

Formatting, default/native all-target Clippy, the full default test suite and
32 native adapter tests passed on Linux. The native release build with
`--future-incompat-report` reported zero dependencies with future-incompatible
warnings. Canonical `cargo llvm-cov --locked --workspace` measured 92.04% line
coverage (148,843 of 161,713), above the unchanged 89% floor; process-heavy
fixtures ran with four test threads outside the execution sandbox.

The complete isolated X11 window acceptance passed. The final release binary
also passed `--window-controls` in standalone and persistent-session modes,
including text/image clipboard round trips, terminal paste, font shortcuts,
PTY geometry and config on reopening. The persistent run additionally verifies
parent editor wait against the matching executable. macOS and Wayland
clipboard/font interaction acceptance remains unverified locally.

## On-demand frames — 2026-10-08

GPUI 0.2.2 is built from the patched copy in `vendor/gpui` (provenance and the
exact changes are in `vendor/gpui/RUNYTE-PATCH.md`). On X11 and Wayland a window
is drawn when it becomes dirty: immediately when no frame was drawn during the
last refresh interval, otherwise one interval after the previous frame, or on
the pending `wl_surface.frame` callback. Idle windows have no refresh timer and
request no frame callbacks. The Vulkan surface on X11 uses its own X connection
so that the driver's presentation thread cannot consume input events from
GPUI's connection. macOS keeps GPUI's display-link frames.

`tests/native_window.py --latency` measures key-to-pixel latency for 60
isolated keystrokes in Insert mode at 120×40 cells (XTest press, `XGetImage`
readback of the edited row) and then counts the window process's context
switches over five idle seconds. Each key must reach the screen within one
second; `--max-idle-wakeups` turns the idle count into an assertion, which CI
sets to 30 per second.

### Measured acceptance

Linux 7.2.8, AMD Radeon 880M (RADV), release builds, a rootful Xwayland server
at 60 Hz inside a headless KWin virtual output (no other clients):

| Build | Median | p90 | Max | Idle context switches |
| --- | ---: | ---: | ---: | ---: |
| Registry GPUI 0.2.2 | 12.6–15.9 ms | 29.2–29.7 ms | 33.2–33.3 ms | 71–73 per second |
| Patched GPUI | 4.6–5.2 ms | 5.7–6.5 ms | 6.3–7.0 ms | 9 per second |

GPUI's rule that re-presents an unchanged scene on every frame request for one
second after input applies only to platforms without on-demand frames, so a
Linux window renders nothing unless something changed.

Per-thread figures for the same build (process CPU time and context switches
from `/proc`, 120 alternating `x`/Backspace keys 100 ms apart on a 1,200-line
Rust file):

| Thread | Window, CPU per key | Terminal frontend in a PTY, CPU per key |
| --- | ---: | ---: |
| Syntax worker | 3.2–3.4 ms | 3.5 ms |
| Host loop | 3.0 ms | 3.2 ms |
| GPUI main thread | 2.0 ms | (terminal emulator, not measured) |

While idle, the GPUI main thread does not wake at all. The window process
wakes about 8.7 times per second, the terminal frontend about 7; both come
from the host's maintenance timers, file and Git monitors, and tokio workers.
A tokio blocking thread present only in the window accounts for the difference
(2 per second). Moving the pointer over an idle window costs about 127 context
switches per second against 468 before, and renders nothing.

An independent probe that reads back only the start of the edited row measured
the patched window at 3.9–4.5 ms median (p90 below 5.3 ms), against 3.8 ms
median for `runyte --ide` in Alacritty on the same display before the change.
A diagnostic GPUI build that polled every millisecond measured 3.3–3.6 ms; it
kept the processor awake with about 1,000 wakeups per second and is not a
usable configuration.

The full isolated X11 acceptance passed with the patched build in standalone,
`--mux`, `--window-controls` and `--window-controls --mux` modes. Persistent
modes need the tested binary first on `PATH` and a short temporary directory
for the workspace socket. The Wayland scheduler has not been exercised at
runtime: the headless compositor offers no input injection, and its virtual
output did not deliver frame callbacks even to the unpatched build, which
painted once and then stopped. It needs a run on a Wayland desktop. macOS is
unchanged.
