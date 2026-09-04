# Fuzzy path matching

Recorded measurements of what Runyte's picker ranking costs and how far its
answers agree with fzf's, on identical candidates. This is a register of
measurements, not of budgets: `tests/performance.rs` holds the assertions that
fail in CI, and nothing here is enforced automatically.

The harness is `benchmarks/fuzzy.py`, and `benchmarks/README.md` documents what
each column means and how the two programs are made comparable. Results are
recorded by running:

```sh
cargo build --release --example fuzzy_filter
benchmarks/fuzzy.py
```

## Reading these numbers

fzf is here as a reference point with a known ranking, not as a target to
match. Where the two disagree, one of them is answering the query better; which
one is a judgement about paths, and the disagreement tables below are what that
judgement is made from.

**`runyte rank only` is the column that corresponds to the editor.** It is the
ranking pass alone, median of several passes inside one process. The picker's
candidates are already in memory from its own scanner, so a keystroke costs
that pass and nothing around it.

**The whole-process columns are two CLI filters, not two editors.** They
include process start, reading the corpus from standard input, and writing the
answer. Runyte's side of that is `examples/fuzzy_filter.rs`, which reads
standard input into one string; the editor never does this. Those columns say
whether Runyte could stand in for fzf as a filter. They are also the noisiest
rows here — on the 100,000-candidate corpus the same cell has moved by more
than 10 ms between runs on an otherwise idle machine, while the rank-only
figure beside it moved by under 1 ms.

**fzf matches on every core.** The `fzf, one thread` column is the same run
under `GOMAXPROCS=1`. Runyte ranks on one background worker, so that column is
the like-for-like one and the default-fzf column is what a person gets.

Absolute values are machine-specific and are not comparable between result sets
taken on different hardware. The corpus is generated from a fixed seed, so it
does not change when Runyte's source does.

## Result set, 2026-09-04

AMD Ryzen AI 9 365, 20 cores, 27 GB, Linux 7.1.9-200.fc44.x86_64, rustc 1.97.1,
release profile. Runyte 0.1.10, fzf 0.74.2, `--scheme=path`. Seven samples per
cell, median reported.

### Cost

Milliseconds.

#### 1,000 candidates

| query | typed | runyte | runyte rank only | fzf | fzf, one thread |
| --- | --- | ---: | ---: | ---: | ---: |
| empty | (empty) | 0.8 | 0.1 | 2.7 | 2.5 |
| one character | `s` | 1.5 | 0.6 | 3.0 | 2.8 |
| segment | `src` | 1.0 | 0.3 | 2.8 | 2.6 |
| acronym | `fpr` | 0.9 | 0.1 | 2.5 | 2.4 |
| across a separator | `keymap` | 0.9 | 0.1 | 2.5 | 2.3 |
| basename | `file_picker.rs` | 0.8 | 0.1 | 2.5 | 2.3 |
| path | `src/parser` | 0.8 | 0.1 | 2.7 | 2.5 |
| two terms | `parser test` | 0.8 | 0.1 | 3.1 | 2.8 |
| no match | `zzqx` | 0.7 | 0.1 | 2.5 | 2.3 |

#### 10,000 candidates

| query | typed | runyte | runyte rank only | fzf | fzf, one thread |
| --- | --- | ---: | ---: | ---: | ---: |
| empty | (empty) | 3.2 | 1.0 | 5.9 | 5.5 |
| one character | `s` | 10.8 | 6.2 | 9.1 | 10.4 |
| segment | `src` | 8.5 | 5.3 | 6.9 | 9.2 |
| acronym | `fpr` | 3.2 | 1.3 | 5.2 | 4.9 |
| across a separator | `keymap` | 3.0 | 1.6 | 4.8 | 4.0 |
| basename | `file_picker.rs` | 2.8 | 1.0 | 4.7 | 3.9 |
| path | `src/parser` | 3.5 | 1.9 | 5.0 | 4.5 |
| two terms | `parser test` | 2.7 | 1.5 | 6.3 | 12.3 |
| no match | `zzqx` | 2.2 | 1.0 | 4.6 | 3.6 |

#### 100,000 candidates

| query | typed | runyte | runyte rank only | fzf | fzf, one thread |
| --- | --- | ---: | ---: | ---: | ---: |
| empty | (empty) | 22.6 | 11.5 | 36.6 | 40.1 |
| one character | `s` | 62.4 | 48.9 | 56.9 | 61.9 |
| segment | `src` | 54.9 | 46.3 | 36.0 | 54.7 |
| acronym | `fpr` | 19.9 | 11.9 | 19.1 | 19.8 |
| across a separator | `keymap` | 17.2 | 8.3 | 14.4 | 17.3 |
| basename | `file_picker.rs` | 12.8 | 7.4 | 12.4 | 22.2 |
| path | `src/parser` | 18.5 | 11.4 | 15.3 | 19.0 |
| two terms | `parser test` | 15.4 | 7.5 | 27.2 | 57.3 |
| no match | `zzqx` | 15.3 | 5.7 | 13.2 | 13.3 |

Ranking is not what a picker keystroke is spent on at the sizes a picker is
normally opened at. At 10,000 candidates every query ranks in under 7 ms and
most in under 2 ms, well inside a frame. The cost is carried by the number of
candidates that match rather than the number scanned: `s` and `src`, which
match 93% and 39% of the corpus, are several times the rest, because a rejected
candidate leaves the filter before it is ever scored.

At 100,000 candidates a single character costs 49 ms of ranking, which is past
a frame. That is the ceiling worth knowing about; the file scanner's own bound
decides whether a corpus that large is reached in practice.

### Agreement, 10,000 candidates

| query | typed | runyte matched | fzf matched | match set | top 10 shared | same first |
| --- | --- | ---: | ---: | --- | ---: | --- |
| one character | `s` | 9,263 | 9,263 | same | 10/10 | yes |
| segment | `src` | 3,879 | 3,879 | same | 10/10 | yes |
| acronym | `fpr` | 564 | 564 | same | 8/10 | yes |
| across a separator | `keymap` | 196 | 196 | same | 8/10 | yes |
| basename | `file_picker.rs` | 63 | 63 | same | 10/10 | yes |
| path | `src/parser` | 191 | 191 | same | 4/10 | yes |
| two terms | `parser test` | 239 | 691 | +0 / −452 | 0/10 | no |
| no match | `zzqx` | 0 | 0 | same | — | no |

The two programs agree closely. On every query but one they accept exactly the
same candidates — identical counts, identical sets — and they put the same
candidate first on all seven queries that have one. Three of the seven share
all ten of their top ten.

`src/parser` shares only four of ten while still agreeing on the first result.
Both programs put the `src/parser` directory at the top; below it they order
the files inside it differently, on scores separated by a few points. This is
the ordinary case of two scorers weighing the same near-equal candidates
slightly differently, not a disagreement about what the query meant.

**Multiple terms mean different things in the two programs.** This is the only
substantive divergence, and it is deliberate. `parser test` matched 239
candidates in Runyte and 691 in fzf, and every one of the 452 extra is fzf's.
Runyte requires each term to appear as a contiguous literal substring, in the
order typed; fzf treats each term as an independent fuzzy subsequence in any
order. So fzf's entire top ten is directories like `test/parser` and
`test/parser/app`, none of which Runyte matches at all, because `test` precedes
`parser` there.

`FuzzyMatcher` documents the rule: two or more terms are matched as themselves,
in order, "because that is what someone typing three words means by them". The
measurement records what it costs — roughly two thirds of the candidates fzf
would offer, and a different set of results entirely for a query of this shape
— rather than proposing that it change.

`parser test` is also the query where fzf gains most from threads: 27.2 ms on
20 cores against 57.3 ms on one, at 100,000 candidates. Runyte ranks it in
7.5 ms on one thread.

## Why directories are in the corpus

The generated corpus contains directories as well as the files under them,
because that is what the picker ranks. This is worth stating because getting it
wrong produces a confident and completely wrong result.

Measured against a corpus of files alone, Runyte appears to rank directory-name
queries far worse than fzf: `src` shares none of fzf's top ten, and
`server2.c` — `s`, `r` and `c` scattered through one name — outranks every file
inside `src/`. The scoring behind that is real. `character_score` awards 30 per
character matched inside the basename and nothing for one matched in a
directory, so a scattered basename match collects 90 points that a contiguous
`src/` match cannot, which is more than the 28-per-pair adjacency bonus can
recover.

None of it reaches the editor, because the directory being searched for is
itself a candidate. `src` has basename `src`, matches the query exactly, and
takes the 10,000-point exact-basename bonus. On Runyte's own tree the finder
answers `src` with `src`, and `app` with `src/app` then `src/app.rs`.

Directories are also grown as a tree rather than drawn per file. A generator
that gives every file a fresh directory chain makes directories more than half
the corpus and leaves no directory query able to match more than a file or two,
which distorts both the timing and the agreement figures.
