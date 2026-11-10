# Settled interaction performance

## 2026-11-10 — Runyte 0.4.4, text and PDF companion panes

This measurement investigates input responsiveness after startup. It does not
replace the [startup measurements](startup-performance.md).

Release binaries are from `50644e33` (0.4.4); the checkout examined is
`d8f33d8e`. The new harnesses are uncommitted additions on that checkout.
The binary hashes match the 2026-11-09 editions run. The machine is Linux
7.2.14-arch1-1 on an AMD Ryzen AI 9 365 with Radeon 880M. The isolated native
display uses KWin/Xwayland; the replacement run confirms `amdgpu` DRM devices.
This does not measure a native Wayland connection or macOS.

### Method

The harnesses are
[`interaction_latency.py`](../../benchmarks/interaction_latency.py) and
[`interaction_pty.py`](../../benchmarks/interaction_pty.py), described in the
[benchmark guide](../../benchmarks/README.md#interaction-latency).

Each case uses a fresh process and isolated workspace, home, configuration,
cache, state and runtime paths. LSP is disabled. The grid is 120×40, with two
side-by-side panes. The left pane edits a 100-line plain-text file. The right
pane contains a second text file or the checked-in two-page PDF fixture;
native PDF cases require evidence that the white page rendered before timing.
Each case alternates 40 leading-space insertions and BackSpace deletions, with
200 ms before each key. Exact saved contents verify both states outside timing.

Native input uses targeted `XSendEvent`; timing ends when `XGetImage` observes
the calibrated changed glyphs beyond the caret. The PTY endpoint is exact
decoded text in a completed synchronized frame and includes Python decoding.
Neither endpoint is physical scanout. PTY results exclude terminal-emulator
rendering and must not be subtracted from native results to isolate overhead.

There are three accepted process runs per case, giving 120 samples per action.
Native case order rotates, and PTY mode order alternates. A one-second CPU
observation before each case must find at most one busy logical CPU, with a
bounded wait if necessary. This does not guarantee absence of interference
throughout a case. No builds ran during measurement.

The [raw artifact](../../benchmarks/results/interaction-2026-11-10.json) retains
all runs, exclusion reasons, quiet checks, binary/harness hashes and pooled
summaries: 960 accepted native actions and 480 accepted PTY actions. Hashes
identify the final delivered harness; the initial native run preceded additions
to untimed diagnostics and failure cleanup, with the same measured key loop.

The first native round overlapped a separate PTY smoke test. That entire round
is retained as diagnostic data and excluded from the reported aggregates,
regardless of its values. A separate full native round replaces it. Every
successful sample in the three accepted rounds is included.

### Native glyph observation

Milliseconds, pooled across the three accepted runs. Median uses the midpoint
of the two central values; p95/p99 use nearest rank. These are descriptive
sample percentiles, not stable tail-latency guarantees.

| Mode | Companion | Insert median | p95 | p99 | Maximum | Delete median | p95 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ide | text | 3.34 | 4.49 | 4.83 | 5.24 | 3.33 | 4.21 |
| ide | PDF | 9.94 | 11.00 | 11.55 | 11.60 | 10.02 | 11.16 |
| mux | text | 4.22 | 5.61 | 5.80 | 6.90 | 4.33 | 5.92 |
| mux | PDF | 9.36 | 11.79 | 12.66 | 12.78 | 9.81 | 11.89 |

The PDF companion adds about 5–7 ms to median insertion in this fixture.
Mux adds about 0.9 ms with a text companion; these results do not establish
an additional mux penalty when the PDF is present. Deletion shows the same
broad pattern. This is settled plain-text editing, not integrated-terminal
input, source-code syntax updates, rapid typing, or PDF page changes.

### Decoded PTY observation

The companion pane contains text in both modes. These numbers include decoding
the terminal stream in Python and use a different endpoint from the native
table above.

| Mode | Insert median | p95 | p99 | Maximum | Delete median | p95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ide | 4.05 | 5.23 | 5.59 | 6.56 | 4.11 | 5.20 |
| mux | 5.67 | 6.84 | 7.50 | 8.23 | 5.74 | 6.63 |

Mux adds about 1.6 ms to median insertion at this endpoint. This supports
investigating persistent-host framing and transport, but does not isolate
JSON encoding, copying, socket transfer, client rendering or decoder overhead.

### Separate CPU cell-paint diagnostic

A separate instrumented run uses 12 insertion/deletion pairs per case and
`RUNYTE_NATIVE_PAINT_TIMING=1`. Only trace lines emitted during the editing loop
are retained, excluding setup and save verification. Other frames within that
interval are included; the trace does not associate each frame with one key.
Instrumented input timings are not mixed into the clean latency tables.

| Mode | Companion | Paint frames | Median CPU paint, ms | Min–max, ms |
| --- | --- | ---: | ---: | ---: |
| ide | text | 28 | 0.447 | 0.149–0.523 |
| ide | PDF | 26 | 7.254 | 3.269–7.626 |
| mux | text | 26 | 0.456 | 0.135–0.517 |
| mux | PDF | 26 | 6.022 | 3.300–7.781 |

The cell painter's median CPU cost rises approximately 13–16 times beside the
PDF. This identifies cell painting as a substantial contributor to the observed
gap. It does not isolate background quads from glyph layers, and these separate
runs cannot be subtracted to allocate every millisecond of end-to-end latency.

### Interpretation and first optimization boundary

Current media rows disable background run merging and use per-glyph layers
for adjacent text. Cached scenes still replay those layers. These are
source-confirmed costs; the diagnostic supports targeting cell painting first,
without claiming that image compositing or presentation contributes nothing.

The first controlled production experiment should merge same-colour background
cells outside raw media/preview footprints while retaining per-cell backgrounds
inside those footprints, including cells reopened by overlays. Keep glyph
layers unchanged initially. GPUI derives depth from intersecting prior bounds,
so merely removing the media guard or grouping every visible span can change
compositing. Preserve the existing cursor, oversized-glyph, italic-overhang,
emoji, decoration and paint-acknowledgement behavior.

Before accepting that change, compare pixels beside PDF/image/preview panes,
including overlays across media boundaries and cached replay, then repeat this
measurement. Existing text-only paint-style comparisons do not establish media
split equivalence. No production rendering change is included in this record.
