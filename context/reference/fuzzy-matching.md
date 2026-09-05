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
benchmarks/fuzzy.py --runs 15
```

## Reading these numbers

fzf is here as a reference point with a known ranking, not as a target to
match. Where the two disagree, one of them is answering the query better; which
one is a judgement about paths, and the disagreement tables below are what that
judgement is made from.

**`runyte rank only` measures the example's sequential scoring and sorting.**
It is the median of several passes on candidates already in memory, excluding
process startup and input/output. The editor's `rank_entries` can score more
than 2,048 candidates across available cores, then sort the merged results on
one thread. The example does not exercise that parallel path or measure the
finder's query-to-results latency.

**The whole-process columns are two CLI filters, not two editors.** They
include process start, reading the corpus from standard input, and writing the
answer. Runyte's side of that is `examples/fuzzy_filter.rs`, which reads
standard input into one string; the editor never does this. Those columns say
whether Runyte could stand in for fzf as a filter. They are also the noisiest
rows here — on the 100,000-candidate corpus the same cell has moved by more
than 10 ms between runs on an otherwise idle machine. The rank-only figure
beside it is steadier but not still: at that size it has moved by several
milliseconds between short runs, which is why this set takes fifteen samples
per cell rather than seven. Read a difference of a millisecond or two at
100,000 candidates as noise unless a rebuild reproduces it.

**fzf matches on every core.** The `fzf, one thread` column is the same run
under `GOMAXPROCS=1`. The Runyte benchmark filter scores and sorts on one
thread. This controls matching parallelism, while the default-fzf column
measures its ordinary configuration. Neither comparison isolates algorithm
cost, and multi-term matching semantics differ as described below.

Absolute values are machine-specific and are not comparable between result sets
taken on different hardware. The corpus is generated from a fixed seed, so it
does not change when Runyte's source does.

## Result set, 2026-09-04

AMD Ryzen AI 9 365, 20 cores, 27 GB, Linux 7.1, rustc 1.97.1,
release profile. Runyte 0.1.10, fzf 0.74.2, `--scheme=path`. Fifteen samples
per cell, median reported.

### Cost

Milliseconds.

#### 1,000 candidates

| query | typed | runyte | runyte rank only | fzf | fzf, one thread |
| --- | --- | ---: | ---: | ---: | ---: |
| empty | (empty) | 0.8 | 0.1 | 2.8 | 2.6 |
| one character | `s` | 1.5 | 0.6 | 3.1 | 2.9 |
| segment | `src` | 1.0 | 0.3 | 2.8 | 2.7 |
| acronym | `fpr` | 0.8 | 0.1 | 2.6 | 2.4 |
| across a separator | `keymap` | 0.8 | 0.1 | 2.6 | 2.4 |
| basename | `file_picker.rs` | 0.9 | 0.1 | 2.6 | 2.4 |
| path | `src/parser` | 0.8 | 0.1 | 2.6 | 2.5 |
| two terms | `parser test` | 0.8 | 0.1 | 3.1 | 3.0 |
| no match | `zzqx` | 0.7 | 0.1 | 2.6 | 2.3 |

#### 10,000 candidates

| query | typed | runyte | runyte rank only | fzf | fzf, one thread |
| --- | --- | ---: | ---: | ---: | ---: |
| empty | (empty) | 3.2 | 0.9 | 6.0 | 5.7 |
| one character | `s` | 10.1 | 7.9 | 8.9 | 10.7 |
| segment | `src` | 8.2 | 5.0 | 7.1 | 9.5 |
| acronym | `fpr` | 3.4 | 1.9 | 5.1 | 4.9 |
| across a separator | `keymap` | 3.1 | 1.6 | 4.9 | 4.1 |
| basename | `file_picker.rs` | 2.8 | 0.9 | 4.8 | 3.9 |
| path | `src/parser` | 3.4 | 1.5 | 5.0 | 4.9 |
| two terms | `parser test` | 4.0 | 1.8 | 6.2 | 11.7 |
| no match | `zzqx` | 2.2 | 1.0 | 4.5 | 3.8 |

#### 100,000 candidates

| query | typed | runyte | runyte rank only | fzf | fzf, one thread |
| --- | --- | ---: | ---: | ---: | ---: |
| empty | (empty) | 26.2 | 11.6 | 30.5 | 33.0 |
| one character | `s` | 62.8 | 49.3 | 57.0 | 72.1 |
| segment | `src` | 60.5 | 47.3 | 41.5 | 58.7 |
| acronym | `fpr` | 20.5 | 11.6 | 18.8 | 24.0 |
| across a separator | `keymap` | 19.1 | 8.3 | 16.5 | 19.0 |
| basename | `file_picker.rs` | 19.5 | 7.5 | 15.5 | 20.8 |
| path | `src/parser` | 20.4 | 11.2 | 16.6 | 21.3 |
| two terms | `parser test` | 20.9 | 13.8 | 25.8 | 59.6 |
| no match | `zzqx` | 13.9 | 5.0 | 14.0 | 16.7 |

At 10,000 candidates every tested query ranks in under 8 ms in the sequential
example, and most in under 2 ms. These are component timings, not measured
interactive response times. The cost is carried by the number of
candidates that match rather than the number scanned: `s` and `src`, which
match 93% and 39% of the corpus, are several times the rest, because a rejected
candidate leaves the filter before it is ever scored.

At 100,000 candidates a single character costs 49 ms of ranking, which is past
a frame. That is the ceiling worth knowing about; the file scanner's own bound
decides whether a corpus that large is reached in practice.

Multi-term queries cost about twice what they did before terms became fuzzy
(`space_separated_query_terms`). Measured on this corpus at the commit before
that change, `parser test` ranked in 7.7 ms at 100,000 candidates against the
13.8 ms above; at 10,000 the two are inside the noise. A term is now an
ordered subsequence rather than a literal run, so it can no longer be rejected
by a substring search that fails on the first character, and 310 candidates
reach scoring where 239 did. Doubled, it is still the query Runyte answers
furthest ahead of fzf: 20.9 ms whole-process against 25.8 on 20 cores and
59.6 on one.

### Agreement, 10,000 candidates

| query | typed | runyte matched | fzf matched | match set | top 10 shared | same first |
| --- | --- | ---: | ---: | --- | ---: | --- |
| one character | `s` | 9,263 | 9,263 | same | 10/10 | yes |
| segment | `src` | 3,879 | 3,879 | same | 10/10 | yes |
| acronym | `fpr` | 564 | 564 | same | 8/10 | yes |
| across a separator | `keymap` | 196 | 196 | same | 8/10 | yes |
| basename | `file_picker.rs` | 63 | 63 | same | 10/10 | yes |
| path | `src/parser` | 191 | 191 | same | 4/10 | yes |
| two terms | `parser test` | 310 | 691 | +0 / −381 | 0/10 | no |
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

**Multiple terms are ordered here and unordered in fzf.** This is the only
substantive divergence and it is deliberate. `parser test` matched 310
candidates in Runyte and 691 in fzf, and every one of the 381 extra is fzf's.
Both programs match each term as a fuzzy subsequence; Runyte additionally
requires the terms in the order typed. So fzf's entire top ten is directories
like `test/parser` and `test/parser/app`, none of which Runyte matches, because
`test` precedes `parser` there. Runyte's own top ten is `parser_test.*` files,
which is the better answer to this particular query.

The counts moved once before, when terms stopped having to be contiguous
(`space_separated_query_terms`): Runyte matched 239 of these candidates when
each term had to appear as a literal run. What remains is the ordering rule
alone, and it is not expected to close.

`parser test` is also the query where fzf gains most from threads: 25.8 ms on
20 cores against 59.6 ms on one, at 100,000 candidates. Runyte ranks it in
13.8 ms on one thread.

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
