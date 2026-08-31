# Startup, quit and idle performance

Recorded measurements of when Runyte first emits shared document content,
then how long it takes to exit, plus what it costs while idle, alongside Neovim
and Helix measured the same way. This is a register of measurements, not of
budgets: `tests/performance.rs` holds the assertions that fail in CI, and
nothing here is enforced automatically.

The harness is `benchmarks/`, and `benchmarks/README.md` documents what each
fixture isolates and how the measurement is taken. Results are recorded by
running:

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
grammar active for `.lua`, but first document content can be emitted before
parsing or highlighting completes. The difference within a same-size pair shows
how language treatment delays that output event, not the complete cost of
parsing the document.

Parser availability was checked for this result set rather than inferred:
Neovim reported an active
tree-sitter highlighter with `filetype=lua` and no regular-expression syntax,
Helix reported its Lua parser and highlight queries present, and Runyte's release
build contains its statically linked Lua grammar and queries. That check makes
the fixture configuration explicit; it does not turn first content emitted into
a parser-completion benchmark.

Absolute values are machine-specific and are not comparable between result sets
taken on different hardware.

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
