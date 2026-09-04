# Benchmarks

Two independent harnesses, each measuring Runyte against a program people
already use, so that a change in Runyte's numbers can be separated from a
change in the machine.

- **`run.py`** — when a terminal editor first emits shared document content,
  how long it then takes to quit, and what it costs while sitting idle, with
  Neovim and Helix measured the same way. Recorded in
  [`context/reference/startup-performance.md`](../context/reference/startup-performance.md).
- **`fuzzy.py`** — what the picker's fuzzy path ranking costs and whether it
  puts the same candidates at the top of the list as fzf, on the same
  candidates. Recorded in
  [`context/reference/fuzzy-matching.md`](../context/reference/fuzzy-matching.md).
  Its own section is [below](#fuzzy-matching-against-fzf).

Both generate their inputs from a fixed seed into `.work/`, which is ignored by
Git. Deleting `.work/` is safe; the next run rebuilds everything in it.

# Startup, quit and idle

## Running

```sh
benchmarks/run.py                  # every editor found, all fixtures
benchmarks/run.py --only runyte    # Runyte alone; no external editors needed
benchmarks/run.py --runs 20        # startup/quit samples; default 10
benchmarks/run.py --idle-runs 7    # independent idle windows; default 5
benchmarks/run.py --no-idle        # skip the idle window
benchmarks/run.py --fixtures long.txt,long.lua
```

Output is Markdown shaped for the results document. Python 3.9 or newer, Linux or
macOS; the idle measurement reads `/proc` and is Linux-only.

Runyte is taken from `target/release/runyte` when that exists and from `PATH`
otherwise. Build it first — a debug binary is not a meaningful measurement:

```sh
cargo build --release
```

Missing editors are skipped with a note rather than failing the run, so
`benchmarks/run.py` is useful on a machine that has none of the others.

## What is measured

**First document content emitted.** Time starts immediately before the
pseudo-terminal fork. The harness records when a stable token from the first
fixture line is emitted in the raw terminal stream. It detects that token across
operating-system read boundaries, so terminal capability exchanges, terminal
setup, and loading presentations cannot be mistaken for document content.
A sample counts only if the process subsequently reaches the quiet guard; an
editor that emits the marker and then exits or crashes is reported as incomplete
rather than as a fast startup.

This metric is deliberately named for exactly what the harness observes. It
does not decode the terminal stream or prove that the rest of the screen has
been presented, test whether the editor accepts input, inspect editor internals,
or wait for work that produces no terminal output. For Runyte, the product's
startup ordering means document text first appears in its complete highlighted
editor frame; that implementation property is not assumed for the other
editors.

**First terminal byte, diagnostic only.** The harness retains the time of the
first byte for diagnosing changes within one editor, but prints it outside the
comparative startup table. Depending on the editor that byte may be an invisible
capability query, terminal setup, a loading presentation, or document drawing.
Those events are not equivalent, so first-byte figures must not be used to rank
editors or described as time to a usable frame. Separately, Runyte's startup
ordering draws a stable, document-free `Opening workspace…` presentation before
it emits document content; the first-byte diagnostic does not identify which
write begins that presentation.

After document content appears, the harness waits until every subsequent output
byte has been followed by 250 ms of quiet before sending the quit command. This
settlement guard is not reported as startup performance: delayed cursor and
capability redraws make it an unreliable proxy for the first document frame.

**Quit time.** After terminal output settles, the harness sends
<kbd>Escape</kbd> <kbd>:</kbd> <kbd>q</kbd> <kbd>!</kbd> <kbd>Enter</kbd> and
records the time from the final keystroke until the editor process exits. The
artificial pauses used to make the sequence behave like real keyboard input are
excluded. Each editor therefore receives the same request to abandon the same
unchanged document; no editor-specific save, plugin, or persistence workflow is
included. Quit medians come from the same samples as startup. A row is reported
as `no measured exit` unless every sample accepts the full command, terminates
before the deadline, exits successfully, and follows settled terminal output,
rather than taking a median of a successful subset or treating a crash as a
fast quit.

**Idle cost.** With a document open and no input, CPU is sampled from `/proc`
over ten seconds, counting the editor and every process it spawned, alongside
the number of times it wrote to the screen. The report gives the median and
range from five independent windows, each using a fresh editor process, so an
intermittent nonzero sample remains visible even when the median is zero. A
fully event-driven editor reports zero for both. `--idle-runs` changes the
sample count and accepts three or more; the report is marked incomplete instead
of taking a median from a subset if any editor exits before its whole window
finishes. Linux process accounting includes both descendants still live at a
sample boundary and the cumulative ticks of children reaped within the window,
so short-lived helpers do not disappear between samples.

CPU sampling is unavailable where `/proc` is absent, including macOS, and is
reported that way rather than as a fabricated zero. Screen writes remain
portable and are still aggregated there.

The idle document is opened inside a Git repository, because that is where an
editor is normally opened and because repository polling is a plausible source
of idle work that would otherwise go unmeasured.

## Fixtures

Generated from a fixed seed into `.work/fixtures/` on first run, and ignored by
Git. They are generated rather than copied from the repository so that a result
does not change when Runyte's own source does. Deleting `.work/` is safe; the
next run rebuilds everything in it.

The matrix is one document at three sizes, written twice. Every editor measured
here has the same single tree-sitter Lua grammar enabled for `.lua`, and no
editor claims a language for `.txt`. The startup metric does not wait for that
parse to complete.

| Size | Lines | On disk |
| --- | ---: | ---: |
| short | 500 | 17 kB |
| medium | 5,000 | 171 kB |
| long | 50,000 | 1.7 MB |

| Fixture | Varies |
| --- | --- |
| `short.txt` | First content from a small document with no language assigned. |
| `medium.txt` | The same event from a realistic working file. |
| `long.txt` | The same event from a large file. |
| `short.lua` | First content from the byte-identical small file with Lua assigned. |
| `medium.lua` | The same language-assigned event from a realistic working file. |
| `long.lua` | The same event where editors' choices about drawing before or after a large parse are visible. |

**The `.txt` and `.lua` files of a size are byte-identical.** Only the extension
differs. The difference between their first-content timestamps therefore shows
how assigning a language affects that output event. It is not the complete
language or parser cost: an editor may emit document text and finish parsing or
highlighting later. The `.txt` file is Lua source that no editor recognises as
such, which makes it a control rather than a second document.

Reading the two axes:

- Across a pair, the difference shows how language assignment affects time to
  the shared output event. It says nothing about later silent work.
- Down a column, the same output event at ten and a hundred times the size shows
  how document size affects time to that event.

The Lua fixture contains no comments, long strings, or calls recognized by any
editor's Lua injection query. All three editors therefore use the Lua grammar
alone. That keeps the setup controlled, but does not imply that parsing has
finished when the marker is emitted.

Rust, Markdown and JSON fixtures were measured previously and have been removed.
Neovim bundles parsers for a fixed short list that does not include Rust, so its
`.rs` figures were regular-expression highlighting of one screen against two
editors parsing the whole document. Markdown is two grammars driven through
injections rather than one, so it reported how the editors compare on Markdown
specifically rather than on source. Both are kept in
[`context/reference/startup-performance.md`](../context/reference/startup-performance.md)
as history and neither is measured now.

## Reading the results

**The startup rows compare one shared output event, not equal completed work.**
Each editor receives the same bytes and the harness timestamps the same token,
but an editor can emit that token before or after parsing, highlighting, or
becoming interactive. The supported cross-editor claim is therefore only that
one editor emitted the shared document content earlier than another. It is not
evidence that the editor was ready, finished more work, or is globally faster.

The fixture configuration is still controlled and recorded:

- The `.lua` rows — Neovim, Helix and Runyte all ship and enable a Lua parser,
  and the fixture triggers no editor's injection queries.
- The `.txt` rows — no editor claims a language.

To check what an editor is actually doing, open the fixture and ask it. For
Neovim:

```sh
nvim --headless FIXTURE \
  -c 'lua local b=vim.api.nvim_get_current_buf()
      print(vim.treesitter.highlighter.active[b] ~= nil, vim.bo.syntax)' -c q
```

`true` with an empty `syntax` is a tree-sitter parse; `false` with a syntax name
is the regular-expression fallback.

**Comparisons of one editor against its own earlier numbers use the same event**
as long as the fixtures, harness, and machine are unchanged. Ordinary run
variation still applies; a small difference is not automatically a code
improvement.

Absolute values are machine-specific. Record the machine alongside any result
set that will be compared against another.

## Harness notes

Two properties of the environment are part of the measurement rather than
incidental to it, and both produced wrong numbers before they were handled.

**Capability queries are answered.** Every editor here emits some combination of
DA1, DA2, the kitty keyboard query, DECRQM mode queries, DSR and OSC colour
requests around its first draw, and at least one will not draw at all until it
gets a reply. The harness replies as a modern xterm would, including during
teardown — an unanswered query at exit reads as a slow quit.

**Keystrokes are staggered.** Writing `ESC : q ! CR` as one block is parsed by
crossterm-based editors as Alt-`:`, because the escape and the byte after it
arrive in the same read. The harness sends the escape alone and pauses, which is
what a keyboard looks like. Quit time begins at the final carriage return, after
this stagger.

Editors run with isolated `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME`,
`XDG_DATA_HOME`, and `HOME` directories under `.work/`, so no personal
configuration, plugins, cache, or state can enter the measurement. Version
probes use the same environment and run from the fixture directory, keeping any
diagnostic file out of the repository root. Neovim additionally runs with
`-n -i NONE` so swap and shada are explicitly outside the startup and quit
measurements. Packaged editor runtimes and grammars remain available; a result
set must still confirm parser availability as described above.

# Fuzzy matching against fzf

`benchmarks/fuzzy.py` is a second, independent harness. It measures what
Runyte's picker ranking costs and whether it puts the same candidates at the
top of the list as fzf does, on the same candidates.

```sh
cargo build --release --example fuzzy_filter
benchmarks/fuzzy.py                    # every size and query
benchmarks/fuzzy.py --sizes medium     # one corpus size
benchmarks/fuzzy.py --runs 15          # more timing samples; default 7
benchmarks/fuzzy.py --no-fzf           # Runyte alone
```

Results are recorded in
[`context/reference/fuzzy-matching.md`](../context/reference/fuzzy-matching.md).

## How the two sides are made comparable

fzf is an interactive program, but `fzf --filter=QUERY` is not: it reads
candidates on standard input and writes its matches to standard output, best
first. `examples/fuzzy_filter.rs` presents `FuzzyMatcher` behind the same
command line, ordering its results the way `file_match_order` orders the
picker's. Both programs are then handed byte-identical bytes on standard input,
and the comparison is between two complete filters rather than between two
functions chosen because they looked alike.

The example is benchmark scaffolding. Nothing in the editor uses it, and it is
excluded from the published crate.

## What the timing columns are

**runyte** and **fzf** are whole processes: start, read the corpus, filter,
write the answer, exit. Nothing is subtracted. A derived "matching only" figure
for fzf would have to be invented rather than measured, and one taken by
differencing against an empty-query run would also difference away the cost of
writing a different number of result lines.

**runyte rank only** is ranking alone, timed inside the process across
`--repeat` passes and reported as their median. It excludes process start,
reading standard input and writing the answer. This is the column that
corresponds to what the editor actually does: the picker's candidates are
already in memory from its own scanner, so a keystroke costs the ranking pass
and nothing around it.

Reading the two together also says what is not being measured. Where the
whole-process figures are close but the rank-only figure is much smaller, most
of both numbers is input handling, and the standard-input path being compared
there is the example's, not the editor's.

**fzf, one thread** is the same fzf run under `GOMAXPROCS=1`. fzf matches on
every available core; Runyte ranks on one background worker. Without that row a
reader cannot tell how much of a difference is the algorithm and how much is
core count.

`FZF_DEFAULT_OPTS`, `FZF_DEFAULT_OPTS_FILE` and `FZF_DEFAULT_COMMAND` are
removed from the environment. Any of them can change fzf's scheme, tiebreak or
matching algorithm, which would make a recorded result impossible to reproduce
anywhere else. fzf otherwise runs with `--scheme=path`, because the corpus is
paths; `--scheme default` is available through `--scheme`.

## What the agreement table is

Three questions that fail separately:

- **match set** — whether the two programs consider the same candidates to
  match at all, ignoring order. This is a property of the filter.
- **top N shared** — how many of one program's first N results appear anywhere
  in the other's first N. This is a property of the ranking, and it is what
  decides whether a person sees the file they wanted without scrolling.
- **same first** — whether the same candidate is first. This is what someone
  who types and presses <kbd>Enter</kbd> without looking gets.

Where the first results differ, the top five from each side are printed, so a
disagreement can be read rather than only counted.

The empty query is measured for timing but left out of the agreement table.
Neither program ranks it — fzf echoes its input order and Runyte's picker sorts
by path — so comparing those two orders would report a ranking disagreement
where no ranking happened.

## Corpus

Generated by `corpus.py` from a fixed seed into `.work/fuzzy/`, and ignored by
Git, for the same reason the startup fixtures are: a benchmark whose input is
Runyte's own tree reports a different number every time that tree changes.

| Size | Candidates |
| --- | ---: |
| small | 1,000 |
| medium | 10,000 |
| large | 100,000 |

The paths are shaped like a source repository rather than drawn from random
strings, since both matchers here are tuned for paths. Segments come from a
small vocabulary, so names repeat across directories and a short query has many
plausible answers to choose between — which is the case a ranking disagreement
shows up in.

**Directories are candidates, not just the files under them**, because that is
what the picker ranks: typing a folder name and being offered the folder is the
ordinary way to reach what is inside it. They are spelled without a trailing
separator, which is how `path_text` spells them for the matcher; the slash in
the picker is added when the row is drawn. Leaving them out once produced a
recorded result claiming Runyte ranked `src` badly, when the editor puts the
`src` directory first.

Directories are grown as a tree rather than drawn per file, so that roughly ten
files share each one, as they do in Runyte's own tree. A generator that gave
every file a fresh chain would both invert that ratio and leave no directory
query able to match more than a file or two.

A synthetic corpus is a controlled input, not a claim about any real tree. A
disagreement seen here is worth confirming against a real checkout, which
`git ls-files` will pipe into either program directly.

## Query shapes

Each row isolates a different part of a score: the empty query as a floor, one
character as the widest possible match set, a directory segment, a scattered
acronym, a query spanning a word separator, a whole basename, a path with the
separator typed, two whitespace-separated terms, and a query nothing matches.

No query uses `^`, `$`, `!`, `'` or `|`. Those are fzf's extended-search
operators and Runyte has no equivalent, so such a query would compare fzf's
syntax against Runyte reading the character literally — not a disagreement
about fuzzy matching.

# Related

`tests/performance.rs` holds in-process budgets for large-document operations.
Those are assertions that fail in CI; the harnesses here are measurements that
are run deliberately and whose results are recorded by hand.
