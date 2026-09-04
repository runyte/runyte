# Space-separated query terms are matched exactly rather than fuzzily

A query containing whitespace is split into terms, and each term must then
occur in a candidate as a contiguous literal run. A query without whitespace is
matched as a fuzzy subsequence. Typing a space therefore replaces fuzzy
matching with exact matching for every term, which is not what a space is
expected to mean.

Terms are also required to appear in the order they were typed and not to
overlap. That ordering rule is deliberate and is to be kept; only the
contiguity of each term is in question here.

## Observed behavior

In the project finder's name mode, over this repository:

| Query | Result | Expected |
| --- | --- | --- |
| `kmap validate` | no results | `src/keymap/validate.rs` |
| `fpick fuzzy` | no results | `context/issues/resolved/file_picker_and_fuzzy_grep.md` |

`kmap` and `fpick` are abbreviations that no candidate contains as a contiguous
run, so the term matches nothing even though the intended candidate is
reachable by subsequence. Both terms in each query appear in the order typed,
so the ordering rule is not what rejects these candidates.

The same abbreviations work as a single term. `fpick` alone finds
`src/file_picker.rs`, and `kmap` alone finds `src/keymap/`. The narrowing
attempt is what loses the match.

fzf returns the expected candidate for both queries.

## Expected behavior

Each space-separated term should be matched as an independent fuzzy
subsequence, and a candidate should match when every term matches it, in the
order the terms were typed.

The ordering rule stays. fzf does not have it — `picker src` finds
`src/picker.rs` in fzf and should continue to find nothing here — so this is a
deliberate divergence rather than an oversight, and the documented example
stating it remains correct.

Whitespace should continue to separate rather than match, and a single term
should continue to mean exactly what it means today: `abc ` asks what `abc`
asks. Smart case should continue to read the whole query, so one capital
anywhere makes every term case-sensitive.

## Constraints

`FuzzyMatcher::matches` decides exactly what `matches_fuzzy` decides and
exactly what `FuzzyMatcher::score` will accept. The scanner's filter, the
picker's narrowing, and the ranker must not disagree about what a match is, so
all three have to change together.

The multi-term branch of `score` is built on the contiguity that would be
removed. Its comment records the assumption: terms are contiguous by
construction, so there is no alignment to search and the only choice is which
occurrence of each term to take. `last_term_start` establishes how far right
each term may sit while leaving room for the ones after it, and `term_start`,
`term_at` and `find_term` locate whole occurrences. With fuzzy terms there is
an alignment to search per term, and those helpers no longer answer the
question being asked.

Scoring scale is part of the same assumption. The multi-term branch adds
`28 * (term.len() - 1)` by hand, which is the adjacency the single-term
alignment in `score_one_term` would have paid for a contiguous run of that
length, so that one term and one word score on the same scale. A fuzzily
matched term is not a contiguous run and that synthetic bonus is no longer the
right correction.

Match positions become non-contiguous. The multi-term branch currently produces
them as `start..start + term.len()` per term. `is_direct_match` and the preview
highlighting consume these positions and read them as runs.

The rule is shared rather than local to one surface. It applies to the finder's
name mode, the finder's content mode, `:fuzzy-grep`,
`:fuzzy-grep-directory`, and the commit picker on `Space g f`, where rows are
ranked as lines rather than as paths.

## Documentation

`docs/user-guide.md` states the current rule in three places, each of which
describes the behavior to be changed:

- The finder's name mode: each space-separated query term is matched against
  any indexed field.
- The space rule itself, which states that two or more words each have to be
  present as themselves. Its worked examples need rechecking rather than
  rewriting wholesale: `src picker` still finds `src/picker.rs` and `picker src`
  still does not, both of which stay correct, but the claims about what the
  rule excludes do not. `src picker` is described as finding `src/picker.rs`
  "without the incidental matches a subsequence through `s…r…c…p…i…c…k…e…r`
  collects", and `content entries` as finding the eighteen lines holding both
  words "rather than 272 lines holding their letters in order". Fuzzy terms
  reintroduce some of what those sentences say is excluded, so both numbers and
  both claims have to be remeasured.
- The commit picker, where spaces separate the query into terms that must
  appear in order. The ordering half of that sentence stays true.

## Notes

The exactness being removed is a narrowing tool, and nothing is proposed to
replace it. fzf reaches the same narrowing through a separate syntax, where a
term prefixed with `'` is matched exactly; Runyte has no equivalent, and
`^`, `$`, `!` and `|` are likewise fzf syntax that Runyte reads literally.
Whether an escape for an exact term is wanted is a separate question from this
report.

Multi-term matching is currently the cheapest scoring path, because a term that
is not present rejects a candidate before anything is scored. At 10,000
candidates the query `parser test` ranks in 1.5 ms and matches 239 candidates,
where fzf matches 691. Per-term alignment replaces an occurrence scan with a
dynamic-programming pass, so both the match set and the cost grow.
`context/reference/fuzzy-matching.md` records the measurement and
`benchmarks/fuzzy.py` re-runs it.

That harness will not show full agreement with fzf afterwards, and should not
be expected to. Its `two terms` row uses the query `parser test`, whose fzf
results are led by `test/parser` and `test/parser/app` — candidates the
retained ordering rule rejects. What should change in that row is the match
count, which is depressed today by contiguity rather than by order; the
remaining gap is the ordering divergence and is intended.
