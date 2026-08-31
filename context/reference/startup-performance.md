# Startup, quit and idle performance

Recorded measurements of how long Runyte takes to begin presenting and reach a
settled editor frame, then exit, plus what it costs while idle, alongside Neovim
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
source does, and a difference between two Runyte columns on the same machine is
a real difference in Runyte.

The fixture matrix is one document at 500, 5,000 and 50,000 lines, written twice
per size as `.txt` and as byte-identical `.lua`. The `.txt` fixtures carry no
language for any editor and isolate reading and drawing. Every editor measured
here parses `.lua` with one tree-sitter Lua grammar, so the difference within a
same-size pair is the cost of treating that document as a language.

Comparisons across editors require them to do the same work. Parser availability
was checked for this result set rather than inferred: Neovim reported an active
tree-sitter highlighter with `filetype=lua` and no regular-expression syntax,
Helix reported its Lua parser and highlight queries present, and Runyte's release
build contains its statically linked Lua grammar and queries.

Absolute values are machine-specific and are not comparable between result sets
taken on different hardware.

## 2026-08-31

Harness `e44e2cf`. Startup and quit are medians of 10 runs, 120x40 pty,
isolated home and XDG storage. Idle is the median and range of five independent
ten-second windows, each with a fresh process. The Runyte release build contains
the first-frame and idle changes recorded by the resolved
`first_frame_and_benchmark_breadth` issue.

- neovim: `NVIM v0.12.4`
- helix: `helix 25.07.1 (a05c151b)`
- runyte: `runyte 0.1.6`

Machine: AMD Ryzen AI 9 365, 20 threads, 27 GB, Linux 7.1.9, btrfs.

Parser availability was rechecked inside the isolated environment. Neovim
reported an active tree-sitter highlighter and `filetype=lua`; Helix reported
its Lua parser and highlight queries present; Runyte contains the statically
linked Lua grammar and queries.

### Startup

First output is the first byte written to begin an editor's presentation; ready
is when substantive drawing goes quiet. The harness does not arm that quiet
test until a substantive-frame byte threshold has been reached. Runyte uses an
intentional document-free startup presentation and does not display document
text until its highlighted editor frame is complete.

| Fixture | Size | neovim first / ready | helix first / ready | runyte first / ready |
| --- | --- | ---: | ---: | ---: |
| `short.txt` | 17 kB | 4 / 17 ms | 16 / 17 ms | 4 / 6 ms |
| `medium.txt` | 171 kB | 6 / 18 ms | 17 / 18 ms | 4 / 7 ms |
| `long.txt` | 1.7 MB | 6 / 20 ms | 18 / 18 ms | 4 / 14 ms |
| `short.lua` | 17 kB | 5 / 28 ms | 23 / 24 ms | 4 / 12 ms |
| `medium.lua` | 171 kB | 5 / 48 ms | 46 / 48 ms | 4 / 27 ms |
| `long.lua` | 1.7 MB | 5 / 175 ms | 222 / 223 ms | 4 / 149 ms |

### Quit

Time from the final force-quit keystroke until the process exits. The harness's
staggered-key delay is excluded.

| Fixture | Size | neovim | helix | runyte |
| --- | --- | ---: | ---: | ---: |
| `short.txt` | 17 kB | 2 ms | 4 ms | 3 ms |
| `medium.txt` | 171 kB | 2 ms | 4 ms | 3 ms |
| `long.txt` | 1.7 MB | 2 ms | 4 ms | 6 ms |
| `short.lua` | 17 kB | 2 ms | 4 ms | 4 ms |
| `medium.lua` | 171 kB | 2 ms | 7 ms | 6 ms |
| `long.lua` | 1.7 MB | 5 ms | 22 ms | 22 ms |

### Idle cost, `medium.lua` open in a Git repository

| Editor | Idle CPU median (range) | Screen writes median (range) |
| --- | ---: | ---: |
| neovim | 0.00 % (0.00–0.00) | 0 (0–0) |
| helix | 0.00 % (0.00–0.00) | 0 (0–0) |
| runyte | 0.00 % (0.00–0.10) | 0 (0–0) |

### Interpretation

Runyte's first output is now 4 ms regardless of the fixture's size or language.
It draws the stable `Opening workspace…` startup presentation immediately after
terminal ownership, then replaces it with the one complete highlighted editor
frame after the same synchronous open and syntax work as before. No document
text is exposed in an unhighlighted or reflowing intermediate state. Its ready
times remain at or below the 2026-08-29 Runyte results and lead all six rows
here.

The opt-in startup trace measured one `long.lua` run as 1.9 ms to terminal
entry, 10.7 ms to the opened buffer, 192.0 ms to completed syntax, and 194.4 ms
to the editor frame. The trace now also distinguishes the startup presentation
from that editor frame. Its buffer-open and syntax milestones surround the
operations they name; previously both were recorded only after the combined
operation returned.

Quit is deliberately retained even where Runyte does not lead. Neovim exits
first in five rows; Runyte and Helix tie on `long.lua`. This category measures
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

First paint is the first byte of output; ready is when drawing goes quiet.

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

Runyte reached a settled frame first in every row. On documents without a
language, its ready time grew from 6 ms at 17 kB to 17 ms at 1.7 MB. The other
editors reached 18–23 ms across the same `.txt` range.

Subtracting each `.txt` ready time from its same-size `.lua` ready time isolates
the measured language cost:

| Size | neovim | helix | runyte |
| --- | ---: | ---: | ---: |
| short | 12 ms | 5 ms | 6 ms |
| medium | 29 ms | 30 ms | 22 ms |
| long | 153 ms | 192 ms | 135 ms |

At 1.7 MB, Runyte settled at 152 ms, against 175 ms for Neovim and 215 ms for
Helix. The corresponding 135 ms language cost shows that syntax work dominates
Runyte's startup on the large fixture; reading and drawing the byte-identical
`.txt` control took 17 ms.

All three editors produced zero screen writes during the unchanged ten-second
idle window. Runyte's single idle sample used 0.30% CPU; because idle is one
window rather than a median, that point estimate should be compared with a
repeated later run before treating a small difference as a regression.
