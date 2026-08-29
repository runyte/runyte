# Startup and idle performance

Recorded measurements of how long Runyte takes to present a settled first frame
and what it costs while idle, alongside Neovim and Helix measured the same way.
This is a register of measurements, not of budgets: `tests/performance.rs` holds
the assertions that fail in CI, and nothing here is enforced automatically.

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
