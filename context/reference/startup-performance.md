# Startup and idle performance

Recorded measurements of how long Runyte takes to present a settled first frame
and what it costs while idle, alongside Neovim and Helix measured the same way.
This is a register of measurements, not of budgets: `tests/performance.rs` holds
the assertions that fail in CI, and nothing here is enforced automatically.

The harness is `benchmarks/`, and `benchmarks/README.md` documents what each
fixture isolates and how the measurement is taken. Results are appended by hand,
newest first, by running:

```sh
cargo build --release
benchmarks/run.py --runs 5
```

## Reading these numbers

Comparisons of Runyte against its own earlier result sets are the intended use.
Fixtures are generated from a fixed seed, so they do not change when Runyte's
source does, and a difference between two Runyte columns on the same machine is
a real difference in Runyte.

Comparisons across editors hold only where each editor is doing the same work.
The clearest confounder is tree-sitter: an editor with no parser installed for a
language highlights the visible window with regular expressions, or not at all,
while an editor with a parser builds a tree over the whole document. Each result
set records what was true of the machine it was taken on. The `.txt` fixtures
carry no language for any editor and are the rows that compare reading and
drawing alone.

Absolute values are machine-specific and are not comparable between result sets
taken on different hardware.

## 2026-08-29 — Lua source fixture

Focused run of the new fixture with the harness at `9aeb784` plus the
uncommitted `large.lua` generator. Median of 5 runs, 120x40 pty, empty config;
idle measurement disabled with `--no-idle`.

- neovim: `NVIM v0.12.4`
- helix: `helix 25.07.1 (a05c151b)`
- runyte: `runyte 0.1.3`, release build from `9aeb784`

Machine: AMD Ryzen AI 9 365, 20 threads, 27 GB, Linux 7.1.9, btrfs.

### Startup

First paint is the first byte of output; ready is when drawing goes quiet.

| Fixture | Size | neovim first / ready | helix first / ready | runyte first / ready |
| --- | --- | ---: | ---: | ---: |
| `large.lua` | 1.0 MB | 6 / 136 ms | 134 / 135 ms | 94 / 95 ms |

### Parser parity and interpretation

All three editors parsed the document with tree-sitter. Neovim reported an
active tree-sitter highlighter, an empty `syntax` option, and a syntax tree with
no error nodes. Helix's installed runtime reported its Lua parser and highlight
queries present. Runyte's release build contains the statically linked
`tree-sitter-lua` grammar and its highlight, injection, and locals queries.

The fixture contains 30,000 complete lines of generated Lua and deliberately
contains no comments, long strings, `cdef` calls, Neovim API calls, or query
sentinels. Those are the constructs matched by the three editors' differing Lua
injection queries, so this row measures one Lua grammar in each editor rather
than a different inventory of injected grammars.

**`large.lua` is the first programming-language row where all three editors do
the same work.** Runyte reached a settled frame in 95 ms, against 135 ms for
Helix and 136 ms for Neovim. Runyte's roughly 30% difference from the other two
is larger than the approximately 10% run-to-run variance observed on larger
fixtures. The 1 ms difference between Helix and Neovim is not signal.

## 2026-08-29

Harness as committed in this record's own commit; the tool reported HEAD
`0a046c6` with the `large.md` fixture still uncommitted. Median of 5 runs,
120x40 pty, empty config.

- neovim: `NVIM v0.12.4`
- helix: `helix 25.07.1 (a05c151b)`
- runyte: `runyte 0.1.3`, release build; `src/` unchanged since `08cb0bf`

Machine: AMD Ryzen AI 9 365, 20 threads, 27 GB, Linux 7.1.9, btrfs.

### Startup

First paint is the first byte of output; ready is when drawing goes quiet.

| Fixture | Size | neovim first / ready | helix first / ready | runyte first / ready |
| --- | --- | ---: | ---: | ---: |
| `small.rs` | 4 KB | 5 / 21 ms | 106 / 106 ms | 22 / 23 ms |
| `small.txt` | 4 KB | 5 / 18 ms | 20 / 20 ms | 6 / 6 ms |
| `medium.rs` | 114 KB | 5 / 21 ms | 141 / 142 ms | 44 / 45 ms |
| `large.rs` | 4.7 MB | 6 / 30 ms | 1986 / 1987 ms | 347 / 348 ms |
| `large.txt` | 4.7 MB | 5 / 24 ms | 22 / 23 ms | 25 / 26 ms |
| `large.md` | 818 KB | 5 / 245 ms | 339 / 340 ms | 136 / 137 ms |
| `minified.json` | 3.7 MB | 5 / 203 ms | 320 / 320 ms | 259 / 260 ms |

### Idle cost, `medium.rs` open in a Git repository, 10 s

| Editor | Idle CPU | Screen writes |
| --- | ---: | ---: |
| neovim | 0.00 % | 0 |
| helix | 0.00 % | 0 |
| runyte | 1.59 % | 66 |

### Which rows compare editors

**`large.md` is the only row where all three do the same work**, and Runyte is
fastest on it: 137 ms against Neovim's 245 ms and Helix's 340 ms. All three
build a tree over the whole document — Neovim's Markdown parser is bundled
upstream, Helix ships one, Runyte links one — and the fixture's fenced blocks
carry no info string, so no editor injects a second grammar the others lack.
This measures Markdown, which is two grammars driven through injections; it is
not a stand-in for large source files.

The `.txt` rows compare reading and drawing alone, since no editor claims a
language for them.

**The `.rs` rows do not compare editors.** Neovim bundles parsers for `c`,
`lua`, `markdown`, `markdown_inline`, `query`, `vim` and `vimdoc` only; Rust is
not among them. Opening `small.rs` reports
`vim.treesitter.highlighter.active` as absent and `&syntax` as `rust`, so those
rows are regular-expression highlighting of one screen while Helix and Runyte
parse the whole document. Neovim's flat ~30 ms on a 4.7 MB file is less work,
not faster work.

Helix's 1987 ms on `large.rs` is also specific to this fixture's shape. Measured
against real source at 12 MB during the same session, Helix read 581 ms and
Runyte 1027 ms — the opposite ordering. Runyte's figures reconcile between the
two shapes once scaled for size; Helix's do not, so the generated fixture's many
small repetitive items appear to be pathological for it. Treat that cell as
evidence about the fixture rather than about Helix.

Run-to-run variance across two full sessions was roughly ten percent on the
larger fixtures even at median-of-five, so differences smaller than that are not
signal.

## Interpretation of the first result set

Recorded 2026-08-29, and expected to remain true until the startup path changes.

Runyte parses a document synchronously before presenting its first frame.
`App::new_with_boundaries` calls `open_launch_targets`, which calls
`parse_buffer` per launch target, which builds a full tree-sitter tree. The
background worker in `src/syntax/background.rs` is spawned afterwards, at
`src/main.rs:3081`, and owns reparsing after edits rather than the initial parse.
Startup therefore scales with document size whenever a language matches, bounded
only by `PARSE_TIMEOUT`, which is five seconds. `INJECTION_LIMIT_BYTES` drops
injection queries above 128 KB but does not otherwise reduce the work.

Two costs separate cleanly from the fixtures:

- The gap between `small.txt` and `small.rs`, 6 ms against 23 ms, is compiling
  one language's queries. The documents are byte-identical, so parsing cannot
  account for it. Query compilation is lazy — `LazyLanguageConfig::get` compiles
  on first use through a `OnceLock`, and registering all built-in languages costs
  well under a millisecond — but the language actually opened pays this cost on
  the main thread before the first frame. An independent measurement with the
  `startup-timing` feature put the same figure at about 15 ms.
- The gap between `large.txt` and `large.rs`, 26 ms against 348 ms, is parsing.
  What remains in `large.txt` is reading the file, which is a floor that
  deferring the parse would not remove.

Deferring the initial parse to the existing background worker would therefore
move roughly 320 ms of this fixture, and proportionally more on larger
documents, off the first frame. `syntax[i] = None` is already a rendered state,
used both for documents in unknown languages and while a reparse is in flight,
so no new presentation state is required. `ParseRequest` currently carries a
prior `DocumentSyntax` and would need a variant that carries text and language
instead.

Runyte's idle cost comes from timed Git refresh rather than from the editor
being awake generally: outside a Git repository the same measurement reports no
CPU and no screen writes. This is tracked separately in
`context/issues/event_driven_git_refresh.md`.
