# Startup and idle benchmark

Measures how long a terminal editor takes to present a settled first frame, and
what it costs while sitting idle. Runyte is measured alongside Neovim and Helix
where those are installed, so a change in Runyte's numbers can be separated from
a change in the machine.

Results are recorded in
[`context/reference/startup-performance.md`](../context/reference/startup-performance.md).

## Running

```sh
benchmarks/run.py                  # every editor found, all fixtures
benchmarks/run.py --only runyte    # Runyte alone; no external editors needed
benchmarks/run.py --runs 9         # more samples per figure
benchmarks/run.py --no-idle        # skip the idle window
benchmarks/run.py --fixtures small.rs,large.rs
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

**Time to a settled first frame.** The editor is spawned on a pseudo-terminal,
and the harness records the first byte of output and the moment output goes
quiet. Small repaints do not restart the settle clock, so an editor that
repaints on a timer still reaches a settled state.

**Idle cost.** With a document open and no input, CPU is sampled from `/proc`
over ten seconds, counting the editor and every process it spawned, alongside
the number of times it wrote to the screen. A fully event-driven editor reports
zero for both.

The idle document is opened inside a Git repository, because that is where an
editor is normally opened and because repository polling is a plausible source
of idle work that would otherwise go unmeasured.

## Fixtures

Generated from a fixed seed into `.work/fixtures/` on first run, and ignored by
Git. They are generated rather than copied from the repository so that a result
does not change when Runyte's own source does.

| Fixture | Isolates |
| --- | --- |
| `small.rs` | Fixed startup cost; the document is too small to matter. |
| `small.txt` | Byte-identical to `small.rs` with an extension no language claims. The difference between the two is the cost of compiling one language's queries. |
| `medium.rs` | A realistic working file. |
| `large.rs` | A document large enough that parsing dominates everything else. |
| `large.txt` | The same bytes with no language. The difference from `large.rs` is parsing; what remains is reading the file. |
| `minified.json` | One very long line, which stresses everything that works outward from the start of a line rather than per row. |

## Reading the results

**Rows are only comparable across editors when each editor is doing the same
work.** The clearest case is tree-sitter: an editor with no parser installed for
a language highlights that file with regular expressions over the visible window,
or not at all, while an editor with a parser builds a tree over the whole
document. Those are different amounts of work and the times are not comparable.
The `.txt` rows exist for this reason — no editor claims a language for them, so
they compare reading and drawing alone.

Before comparing across editors, confirm what each one has available. For
Neovim that means checking whether a parser for the fixture's language is
installed; the version line recorded with each result set does not capture it.

**Comparisons of one editor against its own earlier numbers are sound** as long
as the fixtures and the machine are unchanged, which is what the fixture seed is
for. This is the intended use.

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
what a keyboard looks like. The quit figure has this stagger subtracted.

Editors run with an empty `XDG_CONFIG_HOME`, so no personal configuration or
plugin set is measured. Neovim additionally runs with `-i NONE` so that reading
and writing a shada file is not counted as startup.

## Related

`tests/performance.rs` holds in-process budgets for large-document operations.
Those are assertions that fail in CI; this harness is a measurement that is run
deliberately and whose results are recorded by hand.
