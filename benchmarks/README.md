# Startup, quit and idle benchmark

Measures when a terminal editor first emits shared document content, and
then how long it takes to quit, plus what it costs while sitting idle. Runyte is
measured alongside Neovim and Helix where those are installed, so a change in
Runyte's numbers can be separated from a change in the machine.

Results are recorded in
[`context/reference/startup-performance.md`](../context/reference/startup-performance.md).

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

## Related

`tests/performance.rs` holds in-process budgets for large-document operations.
Those are assertions that fail in CI; this harness is a measurement that is run
deliberately and whose results are recorded by hand.
