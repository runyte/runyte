# Startup, quit and idle performance

Recorded measurements of demonstrated editing readiness, complete file
loading, and initial syntax parsing, alongside Neovim and Helix. Earlier
result sets also record first document output, quitting, and idle cost. This
is a register of measurements, not of budgets: `tests/performance.rs` holds
the assertions that fail in CI, and nothing here is enforced automatically.

The harness is `benchmarks/`, and `benchmarks/README.md` documents what each
fixture isolates and how the measurement is taken. The current readiness
comparison uses `benchmarks/startup.py`; its file-loaded and syntax-ready
columns require the probes documented there. The older first-output, quit,
and idle harness remains available:

```sh
cargo build --release
benchmarks/run.py
```

## 2026-09-09 — complete Linux application workload matrix

Application implementation `f67289c`, measured with harness `104b31f` against
the retained `c7c18bd` release binary. Both builds use
`cargo build --release --locked`. Machine: Linux x86_64,
`7.1.13-200.fc44.x86_64`, AMD Ryzen AI 9 365, 20 logical CPUs,
approximately 27.2 GiB RAM, Rust 1.97.1 and Python 3.14. The measurement ran
serially after builds, tests and coverage finished, on September 9 in
Europe/Warsaw (September 8 UTC).

The [raw artifact](../../benchmarks/results/plugin-applications-2026-09-09.json)
retains all 150 startup samples, 27 workload windows, 240 input latencies,
workload progress checkpoints, binary sizes and SHA-256 identities. Only binary
paths were replaced with repository-relative labels. Binary size increased from
45,335,520 to 48,735,392 bytes (7.50%). The branch SHA-256 is
`4951842e178fc1d591e8e270a7fd258162f6fde1506a7014b85beae28b96e3a1`;
the base is `724984cd14f457584ec0162926dfa0ea74f60bbce10ab7c9f1ef31dedbdbc2ec`.

Every sample uses a fresh temporary Git workspace, isolated home/XDG storage,
disabled LSP, enabled syntax and a 120×40 PTY. Ten samples per startup cell
measure completed document output and a demonstrated first edit separately.
First-byte timing remains diagnostic and is available in the raw artifact.
Epoch 1 readiness is proved by invoking the checked-in uppercase example and
undoing its result outside the captured startup interval. Epoch 2 registration
must be acknowledged by the host. A separate fresh-process series measures the
first rendered, admitted native view following an explicit command.

Values below are median (minimum–maximum), in milliseconds.

| First document output | Base, disabled | Branch, disabled | Epoch 1, quiescent | Epoch 2, quiescent |
| --- | ---: | ---: | ---: | ---: |
| `short.txt` | 7.77 (6.89–10.64) | 8.23 (7.00–9.47) | 8.85 (7.64–10.07) | 8.85 (8.10–9.90) |
| `medium.lua` | 8.34 (7.13–9.87) | 8.90 (7.66–9.90) | 8.97 (7.89–10.55) | 8.72 (6.37–10.32) |
| `long.lua` | 15.00 (12.96–18.02) | 15.13 (13.44–17.63) | 14.59 (12.87–18.33) | 15.99 (13.45–21.55) |

| Demonstrated first edit | Base, disabled | Branch, disabled | Epoch 1, quiescent | Epoch 2, quiescent |
| --- | ---: | ---: | ---: | ---: |
| `short.txt` | 11.51 (10.70–13.98) | 12.57 (11.33–15.73) | 13.36 (11.47–16.90) | 13.09 (11.97–17.56) |
| `medium.lua` | 13.00 (11.40–17.69) | 12.50 (11.33–25.77) | 13.25 (11.96–33.54) | 13.10 (12.06–17.71) |
| `long.lua` | 18.84 (16.85–20.82) | 20.07 (17.26–23.51) | 18.88 (17.93–23.60) | 19.60 (18.02–25.35) |

| Separate native-view startup | Acknowledged registration | First usable view |
| --- | ---: | ---: |
| `short.txt` | 47.95 (44.33–56.99) | 65.96 (61.48–75.41) |
| `medium.lua` | 51.01 (44.82–59.44) | 75.46 (67.20–86.95) |
| `long.lua` | 57.64 (51.44–69.41) | 77.57 (69.03–92.30) |

Disabled first-document medians differ by at most 0.57 ms; demonstrated-edit
medians differ by −0.50 to +1.22 ms, with overlapping ranges. These samples show
no material disabled-startup regression on this machine, without claiming
statistical equivalence or turning the 16 ms sustained-input target into a
startup limit.

Each workload has three ten-second windows after 2.5 seconds of settlement.
CPU includes the editor/host and its descendants, including reaped-child CPU;
100% means one logical CPU. One 10 ms scheduler tick across a ten-second window
is approximately 0.10%, the measurement floor. Output counts observed PTY bytes
and read chunks, not editor write syscalls. A detached job has no frontend, so
its output measurement is unavailable. Reattachment and an authoritative job
query prove the same host still owns the running job after each window.

| Workload | CPU %, median (range) | PTY bytes, median (range) |
| --- | ---: | ---: |
| Base, disabled | 0.00 (0.00–0.00) | 0 (0–0) |
| Branch, disabled | 0.00 (0.00–0.00) | 0 (0–0) |
| Epoch 1, quiescent | 0.10 (0.00–0.10) | 0 (0–0) |
| Epoch 2, quiescent | 0.10 (0.00–0.10) | 0 (0–0) |
| Visible native view | 0.00 (0.00–0.10) | 0 (0–0) |
| 10,000-row, 4 MiB model | 0.00 (0.00–0.10) | 0 (0–0) |
| Detached finite job | 0.00 (0.00–0.00) | unavailable |
| Noisy helper and descendant | 7.00 (6.60–7.30) | 0 (0–0) |
| Repeated maximum-model publication | 60.30 (59.60–60.50) | 2,542 (2,542–2,573) |

The active publisher changes visible content, so its output is expected;
quiescent workloads introduce no output. Publication CPU includes Python model
encoding and all host work and is not an idle cost. Exact canonical model size
and row count are checked after host admission before measurement.

Each active-load case also measures 40 individual edits in each of three
windows. Timing runs from key write through the exact edited marker in a
completed synchronized terminal frame, including harness decoding. Untimed
60–81 ms spacing spans multiple publication cycles. Full saved-document bytes
must equal the original plus all 40 inserted characters. Bounded histories
prove helper output or accepted model commits during the input phase: every
publication phase contains 24 acknowledged commits and every helper phase
contains 15 increasing output checkpoints.

| Input under load, 120 samples each | Median | Range | p95 |
| --- | ---: | ---: | ---: |
| Noisy helper and descendant | 1.64 ms | 1.22–2.60 ms | 2.16 ms |
| Repeated maximum-model publication | 1.64 ms | 1.12–4.08 ms | 2.36 ms |

Both p95 values are below the recorded-machine target of 16 ms. An independent
quiet plugin also answered beside each busy owner. Its end-to-end command
probe took 94.11 ms (94.02–94.74) beside helper output and 95.71 ms
(93.79–96.45) beside publication; these include the harness's fixed 80 ms Escape
guard and command-prompt rendering and are not the per-keystroke metric.

All normal cleanup checks passed, including owned helper/descendant exit.
This completes the Linux release workload matrix; it does not supply macOS
test or coverage evidence. The application plan remains active until its
required supported-platform CI passes. These results do not measure physical
display/audio latency, remote network performance or service-account behavior.

## 2026-09-08 — application API development

Measured on Linux `x86_64-unknown-linux-gnu`, kernel `7.1.13-200.fc44.x86_64`,
AMD Ryzen AI 9 365 (20 logical CPUs, approximately 27.2 GiB RAM), with Rust
1.97.1. The retained base is `c7c18bd`; the measured branch is `e86fe54` plus
the in-progress application implementation. Both were built with
`cargo build --release --locked`. Builds, tests and coverage finished before
the serial measurement. Binary size changed from 45,335,520 to
45,837,312 bytes (1.11%). The retained binary SHA-256 values are:

- Base: `724984cd14f457584ec0162926dfa0ea74f60bbce10ab7c9f1ef31dedbdbc2ec`.
- Branch: `f98af0b21b937bdd56d2135623589a163c02a71381601f7f8ae6055f84a9935d`.

`benchmarks/plugins.py --applications` uses ten launches per startup cell in a
120×40 PTY, isolated home/XDG storage, disabled LSP and enabled syntax. All 120
launches emitted document text, settled and quit successfully. The enabled
cases run the checked-in epoch 1 uppercase example and the epoch 2 jobs example
through the same Python interpreter, with no command/job started during startup.
Values are medians; this harness does not retain startup ranges.

| First document output | Base, disabled | Branch, disabled | Epoch 1, quiescent | Epoch 2, quiescent |
| --- | ---: | ---: | ---: | ---: |
| `short.txt` | 6.1 ms | 5.9 ms | 5.9 ms | 6.3 ms |
| `medium.lua` | 6.2 ms | 6.8 ms | 6.2 ms | 7.0 ms |
| `long.lua` | 12.1 ms | 11.6 ms | 11.9 ms | 12.6 ms |

| Quit after settlement | Base, disabled | Branch, disabled | Epoch 1, quiescent | Epoch 2, quiescent |
| --- | ---: | ---: | ---: | ---: |
| `short.txt` | 3.3 ms | 3.3 ms | 3.8 ms | 3.0 ms |
| `medium.lua` | 5.8 ms | 6.8 ms | 6.8 ms | 5.9 ms |
| `long.lua` | 23.4 ms | 23.6 ms | 22.2 ms | 22.4 ms |

Idle uses three independent ten-second windows after 2.5 seconds of settlement.
CPU includes editor descendants. The native-view case opens `tasks.py` through
the command palette, requires rendered task text, and settles for another 2.5
seconds before measurement. All fifteen idle windows completed. Values are
median (minimum–maximum).

| Idle case | CPU | Screen writes |
| --- | ---: | ---: |
| Base, disabled | 0.10% (0.00–0.10) | 0 (0–0) |
| Branch, disabled | 0.00% (0.00–0.00) | 0 (0–0) |
| Epoch 1, quiescent | 0.00% (0.00–0.10) | 0 (0–0) |
| Epoch 2, quiescent | 0.00% (0.00–0.00) | 0 (0–0) |
| Native task-list view | 0.00% (0.00–0.00) | 0 (0–0) |

These are representative first-document-output and idle measurements. They do
not establish full editing/plugin readiness, input-latency p95, statistical
startup equivalence, large-view/flood performance, detached job cost or the
provider/helper workload matrix. Native macOS evidence is still outstanding.
The [application plan](../plans/active/PLAN_PLUGIN_APPLICATIONS.md) remains active.

The initial real-view check exposed an inherited palette bug: plugin commands
were listed but Enter resolved only built-in names. The corrected implementation
and a rendered-content setup guard are included in this measurement; failed
setups cannot produce zero-cost idle results.

Reproduce after building and retaining both releases, with the benchmark Python
requirements installed:

```sh
python3 benchmarks/plugins.py --before /path/to/base/runyte --applications
```

## 2026-09-07 — experimental process plugins

Measured on Linux `x86_64-unknown-linux-gnu`, AMD Ryzen AI 9 365 (20 logical
CPUs), with Rust 1.97.1. A retained release binary from `ef1bf58` was compared
with the plugin implementation on that base, built with `cargo build --release
--locked`. Builds, tests and coverage finished before measurement. No dependency
was added. The stripped binary grew from 45,093,088 to 45,335,520 bytes (0.54%).

The comparison uses the existing `ptybench.median_startup` and `median_idle`
functions, now exposed for repetition by `benchmarks/plugins.py`. Each startup
cell is the median of ten launches in a 120×40 PTY; every sample emitted the
document marker, reached the quiet guard and quit successfully. Home and XDG
storage were isolated, LSP was disabled, syntax stayed enabled, and the example
case explicitly enabled `docs/plugins/uppercase.py` through the Python interpreter
running the harness. These are representative fixtures, not a complete matrix.

| First document output | Base, plugins absent | Branch, plugins disabled | Branch, example enabled |
| --- | ---: | ---: | ---: |
| `short.txt` (500 lines) | 6.2 ms | 5.5 ms | 6.2 ms |
| `medium.lua` (5,000 lines) | 6.6 ms | 6.7 ms | 6.3 ms |
| `long.lua` (50,000 lines) | 11.1 ms | 11.5 ms | 11.8 ms |

| Quit after settlement | Base, plugins absent | Branch, plugins disabled | Branch, example enabled |
| --- | ---: | ---: | ---: |
| `short.txt` | 2.9 ms | 2.9 ms | 3.6 ms |
| `medium.lua` | 7.0 ms | 6.5 ms | 6.8 ms |
| `long.lua` | 22.7 ms | 22.5 ms | 22.6 ms |

Idle used three independent ten-second windows on `medium.lua`, after the
harness's 2.5-second settlement delay. CPU includes the editor and descendants;
all windows completed. Values below are median (minimum–maximum).

| Idle measurement | Base, plugins absent | Branch, plugins disabled | Branch, example enabled |
| --- | ---: | ---: | ---: |
| CPU | 0.00% (0.00–0.00) | 0.00% (0.00–0.10) | 0.00% (0.00–0.00) |
| Screen writes | 0 (0–0) | 0 (0–0) | 0 (0–0) |

The nonzero disabled-plugin window remains visible rather than being erased by
its zero median. These small samples do not establish a statistically precise
startup difference. They show similar first-document-output medians and no idle
screen writes. First document output does not measure full syntax completion,
input readiness or plugin registration readiness. Plugins start with the optional
host services after the first standalone frame. When disabled they start no task,
process, channel or timer; enabled workers wait on pipes/channels, and buffer
observations share existing host turns rather than adding polling. Work performed
by arbitrary enabled plugins is outside these example measurements.

Reproduce with a retained base release binary and the current release:

```sh
python3 benchmarks/plugins.py --before /path/to/base/runyte
```

## Reading these numbers

Comparisons of Runyte against its own later result sets are the intended use.
Fixtures are generated from a fixed seed, so they do not change when Runyte's
source does. Differences between result sets on the same machine still need to
be judged against ordinary run-to-run variation; a few milliseconds are not by
themselves evidence of a code improvement.

The fixture matrix is one document at 500, 5,000 and 50,000 lines, written twice
per size as `.txt` and as byte-identical `.lua`. The `.txt` fixtures carry no
language for any editor. Every editor measured here has one tree-sitter Lua
grammar active for `.lua`. In the first-output measurements retained below,
document content can be emitted before parsing or highlighting completes.
The difference within a same-size pair shows
how language treatment delays that output event, not the complete cost of
parsing the document.

Parser availability was checked for the historical result sets rather than
inferred: Neovim reported an active tree-sitter highlighter with `filetype=lua`
and no regular-expression syntax,
Helix reported its Lua parser and highlight queries present, and Runyte's release
build contains its statically linked Lua grammar and queries. That check makes
the fixture configuration explicit; it does not turn first content emitted into
a parser-completion benchmark. The new internal probes observe successful
parse completion explicitly; stock-binary readiness requires a rendered and
subsequently verified edit. Readiness and internal milestones come from
separate launches and may complete in different orders across editors. Their
medians cannot be subtracted to isolate a stage's cost.

Absolute values are machine-specific and are not comparable between result sets
taken on different hardware.

The default startup path does not compile a configured keymap: it clones one
`Arc` from the already initialized built-in variant. When a `keys` section is
present, startup compiles and validates both `editor.fast_pane_keys` variants
once so a later settings preview only selects an already diagnosed map. This
bounded, configured-only work has not been added to the measurements below.

The file monitor's two-second reconciliation reads the directory of every open
explorer, because a listing has no cheaper baseline to compare first. The read
happens on the monitor thread and forwards nothing when the listing is
unchanged, so it does not wake the editor. The fixtures below open a document
rather than an explorer, so this work is not in the measurements either.

## Mouse selection autoscroll

Document selection drags at a pane's top or bottom edge schedule one visual-row
scroll every 60 ms in standalone and attached persistent modes. Each tick
prepares the current geometry and resolves the held pointer against the newly
scrolled rows. The deadline exists only during an edge drag; moving inward,
releasing the button, keyboard input, or detachment cancels it. Late ticks
advance once without catching up in a burst. No startup work or idle polling
is added. Startup and idle measurements have not been repeated for this change.

## Git discovery recovery

Failed repository discovery can be retried explicitly through `:git-refresh`.
It uses the existing asynchronous Git service, adds no timer or automatic
retry, and allows only one discovery attempt at a time. Each of the three
discovery reads now uses the existing 30-second local-read deadline and output
bound. Startup and idle measurements below have not been rerun for this change.

## LSP permission startup change

Workspace startup now performs a bounded read of one exact-root permission
record before attaching language services. The manager starts denied and
cannot launch a server until the owning host grants permission. An undecided
workspace with LSP enabled opens a choice overlay. No polling timer is added.
The measurements below predate this change and have not been rerun; readiness
benchmarks now explicitly disable LSP in their isolated configuration while
keeping syntax highlighting enabled. This applies to both `startup.py` and
`run.py` through their shared setup, including instrumented launches. The
first-open choice overlay therefore cannot intercept measurement keystrokes.

## 2026-09-07 — rendered table wrapping

Rendered Markdown pages retain table cell ranges when generated. Column
measurements are shared across a table, including cached tab-width-dependent
measurements. Per-row geometry is reused across navigation and redraws at the
same pane width. Each rendered buffer retains at most 16 table layouts and
2 MiB of their geometry payload; larger layouts are computed without retention.
Viewport snapshots visit visible cell fragments rather than each complete
logical row. No startup scan or polling timer is added.

The shared segment type now also identifies structured table rows. Ordinary
soft-wrap retention remains limited to 8 MiB of segment payload, with its
segment-count ceiling derived from the type's size instead of a fixed count.
Cloned buffers retain immutable table metadata but discard row geometry.

The existing release-mode
`moving_through_wrapped_json_stays_responsive_at_every_depth` check passed on
Linux x86-64 at base `c655304` plus this change. Its 3,277,786-byte JSON fixture
measured slowest movement-plus-snapshot times of 0.58 ms at the beginning,
0.53 ms halfway through, and 0.51 ms near the end, all below its 16 ms budget.
The check ran serially after the correctness and coverage suites completed;
these timings measure ordinary soft wrapping, not table layout construction.

Geometry and interaction coverage lives in `src/table_layout/tests/mod.rs`
and `src/app/tests/markdown_tables.rs`. Startup and idle measurements have not
been repeated for this change.

## 2026-09-07 — soft-wrapped single-line navigation

Measured on AMD Ryzen AI 9 365, Linux x86-64, Rust 1.97.1, with the default
release profile. The baseline is `c56eb96`; the comparison adds cached wrap
geometry, direct rope seeks in snapshot rendering, and contiguous visible
highlight queries. Syntax parsing completes before timing. These are semantic
movement plus snapshot measurements at 120×40, excluding terminal output and
initial layout construction.

An initial probe on the existing 1,693,791-byte `minified_json` fixture averaged
six alternating down/up commands at each location:

| Cursor location | Before | After |
| --- | ---: | ---: |
| Beginning | 16.88 ms | 0.53 ms |
| Halfway through | 85.53 ms | 0.48 ms |
| 2,000 characters before the end | Not measured | 0.49 ms |

The baseline stopped at the middle location when its 50 ms assertion failed.
The end-of-line value is therefore an after-only observation.

Previously, each projected movement recomputed the whole logical line's wrap
boundaries several times. Each visible row then traversed its hidden prefix
through a character iterator. The prefix traversal repeated for every screen
row, making scrolling progressively slower toward the end. Adjacent wrapped
rows also restarted the syntax query separately.

`wrap::Cache` retains at most 16 layouts and 262,144 segments per buffer
(8 MiB of segment payload on a 64-bit target). Its key includes the text
revision, logical row, pane width, and tab width. Clones drop derived geometry;
edits, undo, redo, and text replacement invalidate it through text revisions.
Layouts larger than the retention budget are computed without caching.
The 64,000,000-byte soft-wrap refusal remains in place for first-layout and
resize costs. Startup and idle measurements have not been rerun.

Regression coverage is
`moving_through_wrapped_json_stays_responsive_at_every_depth` in
`tests/performance.rs`: a fixture exceeding 3 MB, beginning/middle/end positions,
48 downward and 48 upward movements per position, with highlighting and cursor
movement asserted and the slowest movement plus snapshot limited to 16 ms.
The final 3,277,786-byte fixture measured a slowest movement of 0.60 ms at the
beginning, 0.54 ms halfway through, and 0.54 ms at 10,000 characters before the
end, over 96 movements per location. All 19 release performance tests passed
serially with `cargo test --release --locked --test performance -- --ignored
--test-threads=1 --nocapture`.

Run the test serially in release mode with the other performance gates. Unicode
coordinates, tab stops, layout eviction, edits and undo are covered in
`src/wrap/tests/cache.rs`; deep Unicode JSON text and highlight preservation
across resizing are covered in `src/snapshot/tests/long_lines.rs`.

## 2026-09-06 — persistent session navigation

Release build from base `9750cd0` plus the session-navigation implementation,
Rust 1.97.1, AMD Ryzen AI 9 365 (20 logical CPUs), Linux 7.1.13. The full binary
hash and individual samples are in
[`session-navigation-2026-09-06.json`](../../benchmarks/results/session-navigation-2026-09-06.json).
Builds, tests, and coverage had finished before measurement.

The new `benchmarks/session_navigation.py` uses a 120×40 PTY, temporary
configuration and workspace roots, disabled LSP, and three samples per scenario.
Cold startup ends at the initial About pane's first output from the last newly
started host. Warm attachment ends at a unique content token in a restored
one-line text document, beyond the styled caret character. These measure visible
output, not demonstrated editing readiness or completed syntax parsing, and are
not directly comparable to the standalone readiness table below.

The attached TUI settles for 32 seconds, covering discovery and the following
attention observation, before a 16-second idle window. CPU sums each editor
process's own user and system time: the attached TUI plus one or three hosts.
It excludes terminal children and is expressed as a percentage of one logical
CPU. Screen writes count PTY reads containing output, with byte totals retained
in the sample file. Zero reads establishes zero output independently of read
chunk boundaries.

| Scenario | Cold start, median | Warm attach, median | Editor CPU, median | Screen writes, each sample |
| --- | ---: | ---: | ---: | --- |
| One host, automatic strip | 32.59 ms | 5.22 ms | 0.00% | 0, 0, 0 |
| Three hosts, automatic strip | 33.32 ms | 6.13 ms | 0.06% | 0, 0, 0 |
| Three hosts, hidden strip | 33.48 ms | 6.37 ms | 0.06% | 0, 0, 0 |
| Three hosts, remote noisy terminal | 33.20 ms | 6.55 ms | 0.25% | 0, 0, 0 |

The noisy case writes a line every 20 ms in another host's terminal. Its host
still consumes and emulates that output, explaining editor CPU above the quiet
cases, while the attached editor emits no unchanged frames. Automatic strip
discovery runs asynchronously every 15 seconds without Git; scalar requests are
bounded and coalesced. Hidden/zen presentation skips attention-only reads, and
detached hosts do not run strip observation. The hidden and quiet medians are
within the timer tick's resolution; these three-sample runs establish a baseline,
not a measured improvement over earlier code. Native macOS was not measured.

The subsequent reattachment fix also requests one asynchronous observation on
each successful frontend attachment, bypassing the retained 15-second deadline.
An in-flight scan from before that attachment is replaced after it completes;
the cached rows remain until the new result arrives. The idle interval and
detached-host behavior are unchanged. The measurements above predate this fix.

## 2026-09-07 — document readiness before initial syntax

Runyte 0.2.0 at `aee17b0` was measured before and after the asynchronous
initial-syntax change. Both runs used the same AMD Ryzen AI 9 365 Linux host,
120×40 PTY, isolated home/XDG configuration, warm-cache fixtures, and ten
measured launches after one discarded warm-up per cell. Editor order rotated.
Builds and tests finished before measurements; desktop applications remained
running. Both comparisons ran in the same execution sandbox.

Neovim was 0.12.5 and Helix was 25.07.1 (`a05c151b`). Rust builds used
1.97.1; the harness used Python 3.14. Runyte readiness used the ordinary release
build. Internal Runyte milestones used a separate disposable build of the
same source with `startup-timing` and the updated observation patch. The patch
observes successful initial parsing on the worker, independently of when a
frame is shown or a result is applied. No current Helix probe build was used.

Every cell below is **median (min–max), milliseconds**. All 180 readiness
samples in each comparison passed whole-file save verification; no successful
slow sample was discarded. The before run also contains 60 Neovim internal
samples; the after run contains 120 Neovim/Runyte internal samples.

### Ready to edit — stock binaries

| Fixture | Runyte before | Runyte after | Neovim after | Helix after |
| --- | ---: | ---: | ---: | ---: |
| `short.txt` | 15.8 (13.2–16.7) | 15.3 (13.7–16.0) | 20.9 (16.8–22.5) | 29.6 (27.9–32.6) |
| `medium.txt` | 17.7 (14.9–25.9) | 16.6 (13.8–18.8) | 20.8 (18.0–25.1) | 30.5 (27.0–34.0) |
| `long.txt` | 24.4 (21.0–27.2) | 23.4 (19.5–27.3) | 23.5 (20.1–26.6) | 34.5 (30.2–47.0) |
| `short.lua` | 21.7 (18.6–23.9) | 17.4 (14.3–25.5) | 38.8 (37.4–44.2) | 38.5 (31.9–40.9) |
| `medium.lua` | 34.0 (30.8–43.6) | 17.6 (15.3–21.7) | 29.3 (27.5–30.8) | 52.7 (49.2–66.7) |
| `long.lua` | 160.9 (153.6–170.4) | 22.3 (20.0–30.0) | 29.7 (26.6–32.3) | 297.6 (289.2–314.6) |

### File loaded and syntax ready — instrumented, after change

| Fixture | Runyte file loaded | Runyte syntax ready | Neovim syntax ready |
| --- | ---: | ---: | ---: |
| `short.lua` | 5.7 (5.1–8.2) | 11.8 (10.7–14.5) | 20.3 (15.8–28.4) |
| `medium.lua` | 6.7 (5.5–8.9) | 26.0 (22.8–30.3) | 33.0 (28.6–42.4) |
| `long.lua` | 11.0 (10.3–16.0) | 156.9 (148.1–180.6) | 163.0 (157.9–182.4) |

The large Lua fixture is ready to edit in 22.3 ms instead of 160.9 ms,
about seven times sooner, and below Neovim's 29.7 ms median in this run.
Runyte's initial parse still completes around 156.9 ms. These measurements
show editing becoming available earlier; parsing itself takes a similar time.
The large plain-text medians are close before and after (24.4 and 23.4 ms),
and the after result is similar to Neovim's 23.5 ms. Overlapping ranges and
ordinary run-to-run variation prevent universal editor rankings. Readiness
and internal milestones come from separate launches and their medians must
not be subtracted to isolate stage costs.

### Quit during initial parsing

`early_syntax_quit.py` decodes the first large Lua document frame and sends
`:q` immediately from Normal mode. Ten measured samples follow one discarded
warm-up, with the same isolated configuration and binaries as above. The
interval starts when the command is sent and ends at successful process exit.
Stock Runyte takes 2.7 ms (2.5–4.7); the instrumented build takes 3.3 ms
(2.6–6.4). None of the ten instrumented launches reported syntax completion
before exit. The [individual early-quit samples](../../benchmarks/results/startup-2026-09-07-async-syntax-early-quit.csv)
retain those observations; the stock binary has no parser-completion probe.

### Settled quit and idle

The existing `run.py` harness, run afterward with ten quit samples per fixture,
measured settled-document quit at 4 ms for `long.txt` and 22 ms for `long.lua`.
The same Lua quit measurement on the pre-change binary was also 22 ms.
These settled results include ordinary cleanup of an accepted document tree;
they do not isolate its destructor cost. Three independent ten-second idle
windows with `medium.lua` open in a Git repository measured 0.00% CPU median
(range 0.00–0.10% of one logical CPU) and zero screen writes in every sample.

### Samples and provenance

The [before samples](../../benchmarks/results/startup-2026-09-07-async-syntax-before.csv)
and [after samples](../../benchmarks/results/startup-2026-09-07-async-syntax-after.csv)
retain every measured value, including plain-text internal milestones.
The [preliminary samples](../../benchmarks/results/startup-2026-09-07-async-syntax-first-pass.csv)
predate the second review's worker-side tree-disposal correction and are
retained separately; their large Lua readiness median was 23.1 ms. That stock
binary's SHA-256 was
`db558ce7051160f4121688b8c7f0434e923a08954dd95737ff6010c5a360445d`, and its
probe's was `8bb12a579eb01ae8f1f9d27dcb554be88a42ca26440e9cad5c606b1b7bf6e1cb`.
The tables above and binary hashes below describe the final reviewed build.

| Binary | SHA-256 |
| --- | --- |
| Runyte before | `b10630314605080e2021192a95ec4740918155d49b0391fafb2fadfd8d7dc160` |
| Runyte after | `f8ebe84e611f4ade1bd4c15834d1fb191dfdffab39de6781823248108225d20d` |
| Runyte after probe | `d85747f556e7bc0d9a3620c667d4ee735a9e77c5341f3634f2d2008eb7649a89` |
| Neovim | `31b1f7b2bbf9d790596e4f9a76f3b7f09c01cf501c314ffa68d365a7fc6a03b0` |
| Helix | `3f31b5db36dec738e153fc027edb280063616df8823598df2da56c80c71542e5` |

## 2026-09-05 — readiness, loading, and syntax

Machine: AMD Ryzen AI 9 365, 10 cores / 20 hardware threads, approximately
27 GiB RAM visible to the OS, Linux 7.1, btrfs. Measurements ran outside the
filesystem/process sandbox after compilation and tests finished. Ordinary
desktop applications remained running.

- Neovim 0.12.4, packaged executable.
- Helix 25.07.1 (`a05c151b`), packaged executable for readiness.
- Runyte 0.1.10 from `8c0bcba`, default release build for readiness.
- Internal Helix and Runyte observations use disposable release builds from
  those source revisions with Rust 1.97.1 and the supplied milestone patches.
  Runyte additionally enables `startup-timing`. Helix uses the installed
  `/usr/lib64/helix/runtime` for both builds.
- Python 3.12.12, pyte 0.8.2, wcwidth 0.8.3. No Lua language server installed.

Each cell reports **median (min–max) in milliseconds**, from ten measured
launches following one discarded warm-up. Editor order rotates each round;
home and XDG storage are isolated; the PTY is 120×40. These are warm-cache
measurements. All 360 measured launches completed, and every readiness sample
passed whole-file save verification. No slow successful samples were removed.
The [individual samples](../../benchmarks/results/startup-2026-09-05.csv) retain
the measurements behind every cell; empty syntax fields for plain text mean
not applicable.

Readiness means inserting **one leading space** and observing the first
document line shift by one cell at its recorded position. After timing ends,
saving to a temporary file must reproduce the whole original document with
exactly that extra space. The insertion keeps Lua valid and introduces no
injected grammar. Readiness uses ordinary binaries; the internal measurements
use separate, unedited launches. The
[methodology and probe setup](../../benchmarks/README.md#startup-milestones)
define the precise observation sites and their overhead.

### Ready to edit — stock binaries

| Fixture | Neovim | Helix | Runyte |
| --- | ---: | ---: | ---: |
| `short.txt` | 22.5 (19.7–24.0) | 29.7 (26.7–33.4) | 15.1 (13.2–31.1) |
| `medium.txt` | 23.3 (20.2–24.1) | 31.6 (27.6–44.2) | 15.5 (13.6–17.2) |
| `long.txt` | 25.3 (22.1–37.7) | 31.1 (28.6–34.4) | 23.0 (20.0–25.1) |
| `short.lua` | 41.0 (39.1–43.1) | 34.7 (31.6–53.5) | 21.2 (17.9–27.6) |
| `medium.lua` | 32.0 (28.6–49.4) | 53.1 (49.3–59.0) | 32.7 (30.8–36.6) |
| `long.lua` | 32.2 (31.7–35.9) | 295.7 (290.9–303.3) | 157.5 (154.8–188.4) |

### File loaded — instrumented

| Fixture | Neovim | Helix | Runyte |
| --- | ---: | ---: | ---: |
| `short.txt` | 11.2 (10.2–12.2) | 15.7 (12.4–18.0) | 5.3 (4.1–6.3) |
| `medium.txt` | 10.9 (9.8–11.7) | 15.5 (12.6–18.5) | 6.3 (4.6–7.2) |
| `long.txt` | 14.1 (12.2–14.6) | 18.0 (15.4–19.6) | 12.8 (10.9–14.7) |
| `short.lua` | 11.7 (10.4–12.5) | 14.7 (12.3–18.2) | 5.1 (4.1–6.3) |
| `medium.lua` | 12.1 (9.3–12.5) | 16.7 (13.3–18.7) | 6.7 (5.3–7.5) |
| `long.lua` | 12.8 (9.6–13.9) | 17.1 (12.4–19.3) | 11.9 (9.9–15.3) |

### Syntax ready — instrumented

| Fixture | Neovim | Helix | Runyte |
| --- | ---: | ---: | ---: |
| `short.txt` | not applicable | not applicable | not applicable |
| `medium.txt` | not applicable | not applicable | not applicable |
| `long.txt` | not applicable | not applicable | not applicable |
| `short.lua` | 18.9 (17.1–22.6) | 18.3 (15.8–22.2) | 8.9 (7.4–11.0) |
| `medium.lua` | 32.8 (29.7–39.9) | 36.8 (32.6–39.8) | 22.8 (19.9–26.6) |
| `long.lua` | 162.7 (160.7–168.7) | 206.8 (199.0–224.4) | 153.1 (144.6–175.1) |

### Interpretation

Runyte has the lowest readiness median on all three plain-text fixtures and
the small Lua fixture. Neovim and Runyte are close on the medium Lua fixture:
32.0 and 32.7 ms, with overlapping ranges. Neovim is substantially earlier on
the large Lua fixture: 32.2 ms against Runyte's 157.5 ms and Helix's 295.7 ms.

The internal observations explain why reading the entire file and being ready
to edit are different questions. All three load the large Lua document into
their native text buffers well before its syntax is ready. Neovim demonstrates
a displayed edit before its separate initial-parse measurement completes.
Runyte's startup ordering prepares syntax before exposing document text.
Readiness also includes processing the insertion and displaying the result;
it is not a timestamp for the end of initial parsing.

The columns do not measure equal background progress, but the readiness
comparison requires the same completed user operation in each editor. It
covers one edit near the start of these fixtures, not general editing latency,
every language, or physical terminal rendering. Internal values include
instrumentation overhead and may reflect compiler and packaging differences.
Because readiness and internal values come from separate launches, subtracting
their medians does not isolate parsing, drawing, or input-processing cost.
Quit and idle figures below remain the earlier dated measurements.

### Provenance

The measurement harness is identified by content because these scripts were
not yet committed when the run was taken:

| File | SHA-256 |
| --- | --- |
| `benchmarks/startup.py` | `3f421fd66508c2d8bbc6d5357b3c9025e0176401d10d99349b06ab4ed0abab9a` |
| `benchmarks/ptybench.py` | `cfb688cdeb6e513ce2b1eb5692669f29ad345ad4acfe039034f583d2a095f037` |
| `benchmarks/neovim_milestones.lua` | `821cbffab00dd7936522e3f8c65eb7f1b7d7187901a0680d3833206f00232c68` |
| `benchmarks/milestone_probe.rs` | `9469f335e4d8d1442066c22657f663e4d338b35b6a330ca45dcaf5de56211157` |

The Rust observation sites are retained in
[`runyte-milestones.patch`](../../benchmarks/runyte-milestones.patch) and
[`helix-milestones.patch`](../../benchmarks/helix-milestones.patch).

| Editor | Stock executable SHA-256 | Probe executable SHA-256 |
| --- | --- | --- |
| neovim | `a13c5dd869d4219852604bfcbe6b9895b78fc03ce162ad45da23c1d280f4b411` | same executable with Lua probe |
| helix | `3f31b5db36dec738e153fc027edb280063616df8823598df2da56c80c71542e5` | `3eb3aea66edc6e4ab09e86315beb8f92c16a9c884956d7ffac70f382000c9ddd` |
| runyte | `c3090c35dd7a81507424ca191876e6e5149bba9b059dd087b25a047e2121bba8` | `49af48cf70b3a8f4ae571e1d4d10ab7fdd080060fb5880ecaeac9375b528a364` |


## 2026-08-31

Startup and quit harness `18a6bb9`; idle harness `e44e2cf`. Startup and quit are
medians of 10 runs, 120x40 pty, isolated home and XDG storage. Idle is the median
and range of five independent ten-second windows, each with a fresh process.
The Runyte release build contains the first-frame and idle changes recorded by
the resolved `first_frame_and_benchmark_breadth` issue.

- neovim: `NVIM v0.12.4`
- helix: `helix 25.07.1 (a05c151b)`
- runyte: `runyte 0.1.6`

Machine: AMD Ryzen AI 9 365, 20 threads, 27 GB, Linux 7.1, btrfs.

Parser availability was rechecked inside the isolated environment. Neovim
reported an active tree-sitter highlighter and `filetype=lua`; Helix reported
its Lua parser and highlight queries present; Runyte contains the statically
linked Lua grammar and queries.

### Startup: first document content emitted

The comparative value is the time from immediately before process launch until
a shared token from the first document line is emitted in the raw terminal
stream. It is a common output event, not proof that the terminal has presented
the whole screen, input is accepted, highlighting is complete, or background
work has finished. A sample counts only if the process subsequently reaches
terminal-output quiet.

| Fixture | Size | neovim | helix | runyte |
| --- | --- | ---: | ---: | ---: |
| `short.txt` | 17 kB | 19 ms | 17 ms | 6 ms |
| `medium.txt` | 171 kB | 21 ms | 21 ms | 8 ms |
| `long.txt` | 1.7 MB | 23 ms | 21 ms | 15 ms |
| `short.lua` | 17 kB | 33 ms | 27 ms | 14 ms |
| `medium.lua` | 171 kB | 27 ms | 48 ms | 28 ms |
| `long.lua` | 1.7 MB | 28 ms | 215 ms | 176 ms |

#### First terminal byte (diagnostic only)

The first byte may be an invisible capability query, terminal setup, a loading
presentation, or document drawing. It is not a readiness metric and is not used
for the comparison above.

| Fixture | Size | neovim | helix | runyte |
| --- | --- | ---: | ---: | ---: |
| `short.txt` | 17 kB | 6 ms | 16 ms | 5 ms |
| `medium.txt` | 171 kB | 7 ms | 20 ms | 5 ms |
| `long.txt` | 1.7 MB | 7 ms | 21 ms | 5 ms |
| `short.lua` | 17 kB | 7 ms | 26 ms | 5 ms |
| `medium.lua` | 171 kB | 7 ms | 47 ms | 4 ms |
| `long.lua` | 1.7 MB | 7 ms | 214 ms | 4 ms |

### Quit

Time from the final force-quit keystroke until the process exits. The harness's
staggered-key delay is excluded.

| Fixture | Size | neovim | helix | runyte |
| --- | --- | ---: | ---: | ---: |
| `short.txt` | 17 kB | 2 ms | 4 ms | 4 ms |
| `medium.txt` | 171 kB | 3 ms | 4 ms | 4 ms |
| `long.txt` | 1.7 MB | 3 ms | 4 ms | 5 ms |
| `short.lua` | 17 kB | 2 ms | 4 ms | 5 ms |
| `medium.lua` | 171 kB | 2 ms | 7 ms | 8 ms |
| `long.lua` | 1.7 MB | 6 ms | 22 ms | 28 ms |

### Idle cost, `medium.lua` open in a Git repository

| Editor | Idle CPU median (range) | Screen writes median (range) |
| --- | ---: | ---: |
| neovim | 0.00 % (0.00–0.00) | 0 (0–0) |
| helix | 0.00 % (0.00–0.00) | 0 (0–0) |
| runyte | 0.00 % (0.00–0.10) | 0 (0–0) |

### Interpretation

Runyte's raw first-byte diagnostic is 4–5 ms, but the harness cannot identify
whether that byte is terminal setup or part of `Opening workspace…`. Runyte
separately draws that stable startup presentation, then replaces it with one
complete highlighted editor frame. No document text is exposed in an
unhighlighted or reflowing intermediate state.

Runyte emitted the shared document token first on all three plain-text fixtures
and `short.lua`. Neovim emitted it 1 ms before Runyte on `medium.lua`, which is
within ordinary run variation, and much earlier on `long.lua`: 28 ms versus 176
ms. That is an output-order result, not evidence that Neovim completed the Lua
parse at 28 ms. The older quiet heuristic observed its later drawing at 175 ms,
while Runyte deliberately withholds document text until its highlighted frame
is complete. No result here supports a claim that one editor is globally
fastest or that the loading presentation improved Runyte's parsing speed.

The opt-in startup trace measured one `long.lua` run as 1.9 ms to terminal
entry, 10.7 ms to the opened buffer, 192.0 ms to completed syntax, and 194.4 ms
to the editor frame. The trace now also distinguishes the startup presentation
from that editor frame. Its buffer-open and syntax milestones surround the
operations they name; previously both were recorded only after the combined
operation returned.

Quit is deliberately retained even where Runyte does not lead. Neovim exits
first in all six rows. This category measures
orderly process teardown after the same unchanged document, not a save or
persistence workflow.

Runyte's idle median is 0.00% with zero writes in every window. One window
rounded to 0.10%. The remaining scheduled work is bounded and named: a
one-second host maintenance wake checks Git fallback/retry, monitor
registration, session-list activity and logging health, while the file monitor
keeps a two-second metadata reconciliation for lost native filesystem events.
Signal delivery and monitor deadlines themselves are event-driven rather than
25 ms polling loops. A termination signal received during synchronous startup
restores the saved terminal state and exits directly, so a blocked file open
cannot strand the terminal in raw mode while waiting for the event loop.

## 2026-08-29

Harness `41cb0bf`. Median of 10 runs, 120x40 pty, empty config. The benchmark was
run outside a filesystem sandbox so each editor could use its ordinary writable
cache, state and local-socket paths; the empty configuration still excluded
personal settings and plugins.

- neovim: `NVIM v0.12.4`
- helix: `helix 25.07.1 (a05c151b)`
- runyte: `runyte 0.1.4`, release build from `41cb0bf`

Machine: AMD Ryzen AI 9 365, 20 threads, 27 GB, Linux 7.1, btrfs.

### Startup

This historical table recorded first terminal byte / drawing quiet under the
then-current heuristic. The first-byte values are diagnostic and must not be
used to rank editors; the labels are retained to preserve the original result.

| Fixture | Size | neovim first / ready | helix first / ready | runyte first / ready |
| --- | --- | ---: | ---: | ---: |
| `short.txt` | 17 kB | 6 / 18 ms | 17 / 18 ms | 5 / 6 ms |
| `medium.txt` | 171 kB | 6 / 17 ms | 19 / 20 ms | 6 / 7 ms |
| `long.txt` | 1.7 MB | 6 / 22 ms | 22 / 23 ms | 16 / 17 ms |
| `short.lua` | 17 kB | 6 / 30 ms | 22 / 23 ms | 10 / 12 ms |
| `medium.lua` | 171 kB | 6 / 46 ms | 48 / 50 ms | 28 / 29 ms |
| `long.lua` | 1.7 MB | 6 / 175 ms | 214 / 215 ms | 150 / 152 ms |

### Idle cost, `medium.lua` open in a Git repository, 10 s

| Editor | Idle CPU | Screen writes |
| --- | ---: | ---: |
| neovim | 0.00 % | 0 |
| helix | 0.00 % | 0 |
| runyte | 0.30 % | 0 |

### Interpretation

Runyte reached drawing quiet first in every row. On documents without a
language, its settled time grew from 6 ms at 17 kB to 17 ms at 1.7 MB. The other
editors reached 18–23 ms across the same `.txt` range.

At the time, subtracting each `.txt` settled time from its same-size `.lua` time
was reported as language cost:

| Size | neovim | helix | runyte |
| --- | ---: | ---: | ---: |
| short | 12 ms | 5 ms | 6 ms |
| medium | 29 ms | 30 ms | 22 ms |
| long | 153 ms | 192 ms | 135 ms |

At 1.7 MB, Runyte reached drawing quiet at 152 ms, against 175 ms for Neovim and
215 ms for Helix. These differences remain historical observations of the old
quiet heuristic. They do not prove parser completion or isolate full parser
cost, which is why the current harness no longer labels them readiness or
language cost.

All three editors produced zero screen writes during the unchanged ten-second
idle window. Runyte's single idle sample used 0.30% CPU; because idle is one
window rather than a median, that point estimate should be compared with a
repeated later run before treating a small difference as a regression.
