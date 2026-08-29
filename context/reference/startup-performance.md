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

## 2026-08-29

Harness as committed in this record's own commit; the tool reported HEAD
`0975487`, which is the commit the harness was added on top of. Median of 5
runs, 120x40 pty, empty config.

- neovim: `NVIM v0.12.4`
- helix: `helix 25.07.1 (a05c151b)`
- runyte: `runyte 0.1.3`, release build; `src/` unchanged since `08cb0bf`

Machine: AMD Ryzen AI 9 365, 20 threads, 27 GB, Linux 7.1.9, btrfs.

**Neovim did no tree-sitter work on the `.rs` rows.** It bundles seven parsers
in `/usr/lib64/nvim/parser/` — `c`, `lua`, `markdown`, `markdown_inline`,
`query`, `vim`, `vimdoc` — and Rust is not among them. Opening `small.rs`
reports `vim.treesitter.highlighter.active` as absent and `&syntax` as `rust`,
so the `.rs` rows are regular-expression highlighting of the visible window
rather than a whole-document parse. Helix and Runyte parse the entire document
on those rows. The three columns are not measuring the same work and the Neovim
figures should not be read as a comparison.

No fixture currently produces a three-way tree-sitter comparison. Markdown would
— all three parse it — and Lua would once Runyte gains that grammar. Adding such
a fixture is the clearest available improvement to this benchmark.

### Startup

First paint is the first byte of output; ready is when drawing goes quiet.

| Fixture | Size | neovim first / ready | helix first / ready | runyte first / ready |
| --- | --- | ---: | ---: | ---: |
| `small.rs` | 4 KB | 6 / 21 ms | 108 / 109 ms | 22 / 23 ms |
| `small.txt` | 4 KB | 6 / 15 ms | 22 / 22 ms | 5 / 6 ms |
| `medium.rs` | 114 KB | 6 / 26 ms | 141 / 142 ms | 59 / 60 ms |
| `large.rs` | 4.7 MB | 5 / 24 ms | 2219 / 2220 ms | 357 / 358 ms |
| `large.txt` | 4.7 MB | 5 / 31 ms | 28 / 29 ms | 28 / 29 ms |
| `minified.json` | 3.7 MB | 5 / 232 ms | 309 / 309 ms | 278 / 278 ms |

### Idle cost, `medium.rs` open in a Git repository, 10 s

| Editor | Idle CPU | Screen writes |
| --- | ---: | ---: |
| neovim | 0.00 % | 0 |
| helix | 0.00 % | 0 |
| runyte | 1.29 % | 66 |

### Reservation on the `large.rs` Helix figure

Helix's 2220 ms is specific to this fixture's shape and should not be quoted as
a general comparison. Measured against real source at 12 MB during the same
session, Helix read 581 ms and Runyte 1027 ms — the opposite ordering. Runyte's
figures reconcile between the two shapes once scaled for size, about 329 ms of
parse here against about 870 ms at 12 MB; Helix's do not, so the generated
fixture's many small repetitive items appear to be pathological for it
specifically.

This does not affect comparing Runyte against its own later result sets, which
is what the fixture seed exists for. It does mean the Helix column is evidence
about this fixture rather than about Helix.

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
- The gap between `large.txt` and `large.rs`, 29 ms against 358 ms, is parsing.
  What remains in `large.txt` is reading the file, which is a floor that
  deferring the parse would not remove.

Deferring the initial parse to the existing background worker would therefore
move roughly 330 ms of this fixture, and proportionally more on larger
documents, off the first frame. `syntax[i] = None` is already a rendered state,
used both for documents in unknown languages and while a reparse is in flight,
so no new presentation state is required. `ParseRequest` currently carries a
prior `DocumentSyntax` and would need a variant that carries text and language
instead.

Runyte's idle cost comes from timed Git refresh rather than from the editor
being awake generally: outside a Git repository the same measurement reports no
CPU and no screen writes. This is tracked separately in
`context/issues/event_driven_git_refresh.md`.
