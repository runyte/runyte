# Settled interaction performance

## 2026-11-11 — Component fixes and validation

Tracking: [transport and copies #111](https://github.com/runyte/runyte/issues/111),
[scene replay #112](https://github.com/runyte/runyte/issues/112),
[row preparation #113](https://github.com/runyte/runyte/issues/113), and
[terminal typing tails #114](https://github.com/runyte/runyte/issues/114).
The [component artifact](../../benchmarks/results/interaction-followup-2026-11-11.json)
retains samples, quiet checks, stage traces and provenance. These are component
measurements, not a final end-to-end comparison of the fixes.

The terminal-damage shell comparison unnecessarily cloned both panes before
discarding their terminal fields. An exhaustive borrowed comparison preserves
fallback and title handling. In a same-binary release comparison, median shell
cost fell from 44.932 to 0.419 microseconds for one 120×40 terminal, and from
487.487 to 0.599 microseconds for four 240×80 terminals. Dimensions are per
terminal; each fixture also contains an unchanged document sibling. Ordinary
document typing rejects this delta path earlier. Its synthetic response clone
cost was only 3.939–8.357 microseconds, so transport ownership is unchanged.

Recorded GPUI primitives already passed clipping and retain their bounds and
masks. Replaying them inside an active layer can reuse that layer's order without
repeating the intersection. The experimental implementation passed semantic
comparisons and reduced clear-plus-replay time for 4,800 synthetic glyphs from
119.339/126.380/138.750 to 72.286/80.320/94.154 microseconds for 1/2/4 partitions
per row. The production implementation shares primitive insertion code; its
post-refactor timing remains deferred. Production semantic equivalence and all
12 exact pixel comparisons passed.

Retaining a previous grid snapshot and changing one cell costs 0.709 microseconds
at 120×40 and 2.083 microseconds at 240×80. Whole native preparation measured
50–74 microseconds in separate traces; coverage construction is only part of
that stage. These measurements do not justify a storage or coverage-cache
redesign for the measured typing workload. Wider grids and sustained terminal
output may warrant separate investigation.

The first clean topology comparison stopped at a failed quiet check after a
competing build began; it is retained as excluded data. A repeated ABBA
comparison was also interrupted by competing compilation. Its partial passes
are retained for diagnostics only. Final end-to-end measurements are deferred;
no terminal-tail improvement is claimed.

Formatting, core and desktop all-target Clippy, 5,362 core tests, 172 desktop
tests, and 23 Python harness tests passed. Canonical Linux core line coverage is
92.58%, above the 89% floor. The production replay semantic check and all 12
text/PDF/PNG/HTML pixel comparisons passed. Astra Medium reviewed the changes
and measurement interpretation. Other operating systems were not validated
locally. Final production component timing and end-to-end latency remain
deferred; these correctness checks do not establish a typing-latency gain.

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

At this baseline, media rows disable background run merging and use per-glyph layers
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
split equivalence. No production rendering change was included in that initial measurement.


## 2026-11-10 — Pane-row segmentation experiment

The implementation carries drawable pane areas from both standalone and attached
snapshots into the native frame. Each logical row is divided at pane and raw
media/preview boundaries. Unchanged spans retain their GPUI scenes when another
span changes, while the row retains only its current cell allocation. Styles
still rebuild with one whole-row scan when the row changes, preserving wide-cell
advancement across boundaries. The grid's row COW and host protocol are unchanged.

Background merging and grouped glyph layers now apply outside raw media
footprints. Overlay cells inside a media footprint retain per-cell painting;
cursor-containing spans and oversized/zero-advance glyph spans retain their
fallback. This measures the combined segmentation, cache reuse and localized
batching change, not the independent contribution of each part.

### Controlled comparison

The [raw artifact](../../benchmarks/results/pane-rows-2026-11-10.json) contains
all per-pass samples and summaries, source/binary hashes, quiet checks, document
proofs, separate paint traces and pixel-comparison hashes. The baseline is the
preserved release binary from the initial experiment. Production sources,
manifests and vendor code are unchanged between its source `50644e33` and the
new change's base `fcde99de`.

After builds and tests completed, four sequential passes used baseline →
optimized → optimized → baseline order. Each pass used fresh processes for
IDE/mux × text/PDF/terminal, with 30 insertion/deletion pairs per case. This gives
60 observations per operation/version/case and **1,440 accepted actions**.
Every accepted case passed its quiet check and whole-document verification;
no successful samples were removed. Earlier harness-development checks during
builds are not part of these comparison passes.

The display, geometry, font, input delivery and glyph-observation endpoint are
the same as above. The terminal companion is a synthetic 30-row display refreshed
once per second, with visible output changes verified during measurement. It is
not htop, and keys are not synchronized to its refreshes. These results do not
isolate latency specifically when input coincides with a terminal update.

Milliseconds, pooled medians and nearest-rank p95. Per-pass values remain in the
artifact because pooling can obscure run-to-run variation. With 60 observations,
p99 equals the maximum and is not a useful independent tail estimate here.

| Mode | Companion | Insert median before → after | Insert p95 before → after | Delete median before → after | Delete p95 before → after |
| --- | --- | ---: | ---: | ---: | ---: |
| ide | text | 3.79 → 3.33 | 4.55 → 4.35 | 3.50 → 3.68 | 4.75 → 4.78 |
| ide | pdf | 10.21 → 3.09 | 12.08 → 4.38 | 10.26 → 3.36 | 12.33 → 4.43 |
| ide | terminal | 3.47 → 3.69 | 4.77 → 6.14 | 3.50 → 3.67 | 5.12 → 6.22 |
| mux | text | 4.43 → 4.81 | 6.36 → 6.20 | 4.22 → 4.85 | 5.97 → 6.35 |
| mux | pdf | 9.00 → 4.13 | 11.86 → 5.32 | 9.01 → 4.27 | 11.93 → 5.82 |
| mux | terminal | 5.88 → 6.19 | 8.50 → 8.04 | 6.33 → 6.39 | 8.59 → 8.68 |

The PDF gain is repeatable in both passes: insertion medians fall **69.8% in IDE**
and **54.1% in mux**; deletion medians fall 67.3% and 52.5%. IDE insertion pass
medians are 10.462/10.011 ms before and 2.938/3.142 ms after; mux is
10.819/7.982 ms before and 4.353/3.902 ms after.

There is **no consistent text/terminal latency improvement**. IDE-terminal
insertion p95 worsens in both optimized passes: 7.581/5.450 ms versus
4.774/4.805 ms before. Its pooled insertion p95 rises from 4.77 to 6.14 ms;
deletion p95 rises from 5.12 to 6.22 ms. Mux-text pooled medians rise 8.6% for
insertion and 14.9% for deletion, but the baseline insertion pass medians also
vary substantially, from 5.030 to 3.657 ms. These observations must not be
presented as a general speedup or dismissed as proven measurement noise.

### Separate CPU painting diagnostic

Separate instrumented runs use eight pairs per case. All paint frames emitted
during the editing loop are retained, including terminal-refresh frames. These
are small diagnostic samples, not per-key attribution; their instrumented input
timings are excluded from the latency comparison.

| Mode | Companion | Frames before / after | Median paint CPU before → after, ms |
| --- | --- | ---: | ---: |
| ide | text | 20 / 19 | 0.348 → 0.534 |
| ide | pdf | 19 / 19 | 5.330 → 0.457 |
| ide | terminal | 22 / 23 | 0.458 → 0.543 |
| mux | text | 18 / 18 | 0.456 → 0.538 |
| mux | pdf | 18 / 18 | 4.790 → 0.364 |
| mux | terminal | 22 / 22 | 0.385 → 0.312 |

PDF-case median paint CPU falls roughly 91–92%. Text-only paint medians rise
in this diagnostic. More segment layers and replay work are plausible costs of
the general split, but these measurements do not isolate that mechanism or
explain every millisecond of the terminal tail regression. The evidence favors
localizing media's conservative painting rules; a universal pane-row speedup
has not been established. Retaining the general split is a performance tradeoff,
not an unconditional improvement.

### Correctness and validation

The final release binary matches the baseline exactly in 12 fixed-body pixel
comparisons: styled terminal text, a finder overlay crossing both panes, and the
restored scene, beside text, PDF, PNG and an HTML preview. The fixture includes
italic icons, decorations, emoji and wide-cell placement near the pane divider,
and proves media and the overlay appeared. Missing CJK fallback fonts still
limit verification of actual CJK glyph appearance. Other font metrics, native
Wayland, Windows and macOS rendering were not measured.

Regression tests cover neighboring scene retention, geometry invalidation,
wide-cell predecessors crossing a boundary, and distinct backgrounds for
multiple overlay cells reopened inside media. Required formatting, default and
desktop all-target Clippy, the full default Rust suite, and all desktop package
tests passed. The 23 focused Python harness tests and exact pixel checks passed.
No core Rust source or coverage threshold changed; canonical core coverage was
not rerun for this native-only change. Astra Medium reviewed the implementation,
measurement method and result interpretation.
