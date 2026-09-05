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

## 2026-09-05 — readiness, loading, and syntax

Machine: AMD Ryzen AI 9 365, 10 cores / 20 hardware threads, approximately
27 GiB RAM visible to the OS, Linux 7.1.9-200.fc44.x86_64, btrfs. Measurements ran
outside the filesystem/process sandbox after compilation and tests finished.
Ordinary desktop applications remained running.

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

Machine: AMD Ryzen AI 9 365, 20 threads, 27 GB, Linux 7.1.9, btrfs.

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

Machine: AMD Ryzen AI 9 365, 20 threads, 27 GB, Linux 7.1.9, btrfs.

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
