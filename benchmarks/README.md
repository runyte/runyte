# Benchmarks

Independent harnesses measure Runyte against programs people already use, so
that a change in Runyte's numbers can be separated from a change in the machine.

- **`startup.py`** — demonstrated readiness to edit on ordinary editor binaries,
  plus file-loaded and syntax-ready milestones in separate instrumented runs.
  This supplies the README's editor comparison.
- **`run.py`** — when a terminal editor first emits shared document content,
  how long it then takes to quit, and what it costs while sitting idle, with
  Neovim and Helix measured the same way. Recorded in
  [`context/reference/startup-performance.md`](../context/reference/startup-performance.md).
- **`fuzzy.py`** — what the picker's fuzzy path ranking costs and whether it
  puts the same candidates at the top of the list as fzf, on the same
  candidates. Recorded in
  [`context/reference/fuzzy-matching.md`](../context/reference/fuzzy-matching.md).
  Its own section is [below](#fuzzy-matching-against-fzf).

The harnesses generate their inputs from a fixed seed into `.work/`, which is
ignored by Git. Deleting `.work/` is safe; the next run rebuilds everything in it.

Recorded startup samples are retained under [`results/`](results/) so the
published medians and ranges can be checked without repeating a timing run.

# Startup milestones

```sh
python3 -m venv benchmarks/.work/venv
benchmarks/.work/venv/bin/pip install -r benchmarks/requirements.txt
cargo build --release --locked
benchmarks/.work/venv/bin/python benchmarks/startup.py --runs 10 \
  --json benchmarks/.work/startup-samples.json
```

The JSON retains every measured sample and binary hashes. The Markdown report
gives median and min–max; an incomplete sample invalidates that cell and makes
the command fail. One warm-up per editor, fixture, and measurement mode is
discarded. Editor order rotates each round to reduce fixed-order bias. These
are warm-cache launches, not cold-boot disk measurements. Run on an otherwise
idle machine, after builds finish; background compilation can dominate these
small timings. Do not discard slow successful samples.

All three milestones start immediately before the PTY fork, but readiness and
internal milestones come from **separate processes**. They must not be
subtracted from each other as if they were stages of one recorded launch.

| Milestone | Completion evidence |
| --- | --- |
| File loaded | The complete decoded document is in the editor's native text buffer. Neovim reports `BufReadPost`; Helix reports after constructing `Document` from the loaded rope; Runyte reports `InitialBufferOpened`. |
| Syntax ready | The initial whole-document Lua parse has completed. Neovim's normal parse returns or calls its completion callback with an error-free root reaching the fixture's final newline; Helix has successfully constructed `Syntax`; Runyte's initial `parse_buffer` returns a syntax value. Plain text is reported as not applicable. |
| Ready to edit | After the first document line appears, the harness sends `i` followed by one space. At the recorded starting position of `local function scan_0`, it must now see ` local function scan_0`. It waits for any synchronized-update frame to end, then records the timestamp. |

Readiness uses the normal editor binaries with no internal probes. The harness
does not wait for terminal silence or syntax completion before typing. After
the timed interval, it saves to a temporary path, verifies that the result is
exactly the original **whole file** with one leading space, and requires a
successful quit. An echoed key, status message, unchanged buffer, truncated
file, or crash cannot pass verification. The original fixture is never saved
over. Save and quit costs are outside the readiness value; `run.py` still
measures quitting an unchanged document separately.

This is demonstrated editing readiness for one insertion near the start of a
file, including the harness's screen decoding and input/output round trip. The
single whitespace edit keeps Lua valid, triggers no additional grammar, and
avoids accumulating the cost of multiple typed characters. It does not
identify the earliest instant the editor could have accepted queued
input, test every editing command, or measure physical terminal presentation.
Syntax readiness means a completed parse, not highlighting every off-screen
line, language-server readiness, or absence of later background work. The Lua
fixtures trigger no injected languages.

## Instrumented file-loading and parsing measurements

The Neovim probe is loaded with `--cmd` in internal runs only. It observes the
normal parser's synchronous return or asynchronous completion callback without
requesting a parse or changing that choice. It obtains the Lua parser at
`BufReadPost`, so probe initialization and callback overhead are part of the
instrumented result.

Helix and Runyte need disposable release builds. The supplied patches add only
milestone observations; they are not changes to the distributed editors. The
Helix patch is based on `a05c151b` (25.07.1); the Runyte patch is based on this
repository's `8c0bcba` source. Recheck the observation sites if either patch
needs adapting to a later source revision.

From the Runyte repository root, with a Helix checkout available:

```sh
mkdir -p benchmarks/.work/runyte-probe benchmarks/.work/helix-probe
git archive 8c0bcba | tar -x -C benchmarks/.work/runyte-probe
git -C /path/to/helix archive a05c151b | tar -x -C benchmarks/.work/helix-probe
git apply --directory=benchmarks/.work/runyte-probe benchmarks/runyte-milestones.patch
git apply --directory=benchmarks/.work/helix-probe benchmarks/helix-milestones.patch
cp benchmarks/milestone_probe.rs benchmarks/.work/runyte-probe/src/benchmark_probe.rs
cp benchmarks/milestone_probe.rs benchmarks/.work/helix-probe/helix-view/src/benchmark_probe.rs
cargo +1.97.1 build --manifest-path benchmarks/.work/runyte-probe/Cargo.toml \
  --release --locked --features startup-timing
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo +1.97.1 build \
  --manifest-path benchmarks/.work/helix-probe/Cargo.toml --release --locked
benchmarks/.work/venv/bin/python benchmarks/startup.py --runs 10 \
  --runyte-probe benchmarks/.work/runyte-probe/target/release/runyte \
  --helix-probe benchmarks/.work/helix-probe/target/release/hx \
  --helix-runtime /path/to/installed/helix/runtime \
  --json benchmarks/.work/startup-samples.json
```

Use fresh extraction directories. The runtime must match the Helix source and
contain its Lua grammar and queries; the recorded Linux run used
`/usr/lib64/helix/runtime`. The same runtime is used for stock and instrumented
Helix. Without a probe build, internal cells say unavailable; first text is
never substituted. Missing Lua syntax events fail rather than becoming zero.

Internal events contain timestamps on the shared system wall clock, captured
before appending to a fresh temporary event file. The harness compares elapsed
wall and monotonic time and rejects a clock jump greater than 5 ms. File-loaded
event I/O and probe code add overhead before the syntax event; these are
instrumented application timings, not isolated parser microbenchmarks. Stock
and probe builds can also differ because of compiler or packaging choices.
Record their source revisions, build profiles, runtime, compiler, and hashes
with the result. Use stock-binary readiness for the README comparison.

Verification:

```sh
benchmarks/.work/venv/bin/python -m unittest discover -s benchmarks
```

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
reading standard input and writing the answer. This isolates scoring and
sorting on candidates already in memory. It is not the editor's query-to-results
latency: the example ranks sequentially, while the editor's `rank_entries` can
divide scoring across available cores above 2,048 candidates. The editor sorts
the merged results on one thread and also coordinates discovery, query updates,
and presentation.

Reading the two together also says what is not being measured. Where the
whole-process figures are close but the rank-only figure is much smaller, most
of both numbers is input handling, and the standard-input path being compared
there is the example's, not the editor's.

**fzf, one thread** is the same fzf run under `GOMAXPROCS=1`. fzf matches on
available cores; the Runyte benchmark filter scores and sorts on one thread.
This column controls matching parallelism; it does not isolate algorithm cost
from process startup, input/output, sorting, or differences in matching
semantics. The editor's parallel scoring is not exercised by this benchmark.

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
