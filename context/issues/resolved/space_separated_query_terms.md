---
title: "Space-separated query terms are matched exactly rather than fuzzily"
status: resolved
reported: 2026-09-04
resolved: 2026-09-04
commit: ff87997
---

## Resolution

Fixed by `ff87997` (`Match each space-separated query term fuzzily`).

`FuzzyMatcher::matches` had two branches. One term took the fuzzy-subsequence
path; two or more went through `find_term`, which walks the candidate looking
for the term as a contiguous run and returns where it ends, and each term was
then sought in what the previous one left. `matches_fuzzy` held a second copy
of the same split so the content scanner could apply it without allocating.
Typing a space therefore did not narrow a fuzzy search, it replaced it with an
exact one.

Every term is now matched as a fuzzy subsequence. The requirement that the
terms appear in the order typed is deliberate and was kept, which is a known
divergence from fzf.

The change is smaller than the report suggests because ordered fuzzy terms are
not a new question. An ordered subsequence of `ab` followed by one of `cd` is
an ordered subsequence of `abcd`, so which candidates match is decided entirely
by the terms run together. `FuzzyMatcher` now holds that concatenation as
`query`, `matches` is one call to a shared `subsequence` helper, and
`matches_fuzzy` calls the same helper over `split_whitespace().flat_map(chars)`.
The two can no longer disagree, which is the invariant the old `matches` doc
comment asserted and two implementations had to maintain by hand.

Because the alignment is now the only scorer, the term-occurrence search left
`score` entirely, and with it `find_term`, `term_at`, `term_start` and
`last_term_start`. So did the `28 * (term.len() - 1)` the multi-term branch
added by hand, which existed to put a term back on the same scale as the
adjacency a word of that length would have earned in the alignment.

What a space still decides is scoring, and this is the one piece of new
machinery. `boundaries` marks which characters of `query` open a term, and at
those characters `score_one_term` takes the best predecessor anywhere earlier
at no charge, rather than paying the adjacency bonus or the gap penalty. The
distance between two terms is not a gap: whitespace is what someone types to
say the stretches are separate. Nothing is paid for terms landing adjacent
either, so on a candidate where both alignments sit on the same characters
`abcd` scores exactly 28 above `ab cd`.

Deliberate deviations from what the report asked for: none on the matching
rule. The report asked for fzf's behaviour and fzf does not order its terms;
the ordering rule was retained by decision, so `picker src` still finds nothing
where fzf finds `src/picker.rs`.

One consequence the report did not anticipate: no insertion can widen a result
set any more. Adding a character leaves the earlier ones in place and in order,
so the terms run together only grow, and adding a space does not change them at
all. `splitting_a_term_reconsiders_paths_the_whole_term_excluded` covered an
edit that can no longer widen and became
`inserting_into_the_query_can_only_narrow_what_matched`. `insert_query` still
rescans rather than narrowing on a mid-query edit, which is now conservative
rather than required, and was left alone.

`is_direct_match` was left unchanged by that commit and should not have been.
It counted runs of consecutive positions across the whole query and called the
match direct when there were no more runs than terms, which was sound only
while every term was a contiguous literal. Once a term is itself fuzzy, a gap
inside one term cancels against the next term landing adjacent: `ab cd` against
`a_bcd` matches at `[0, 2, 3, 4]`, two runs for two terms, and was rendered in
the primary emphasis colour although `ab` is plainly scattered. The review
follow-up, `Read directness per term and refuse unanswered filter runs`, reads
the term boundaries from the query instead and requires each term's own slice
of the positions to be consecutive. Terms landing next to each other stay
direct, which is still the tightest a multi-word query can land.

Tests, all in `src/file_picker.rs`:

- `a_query_of_several_words_asks_for_each_of_them` — a term is as loose as a
  lone word, terms are still ordered and all still required, smart case still
  reads the whole query, and a split term accepts what the joined term accepts.
- `each_term_of_a_several_word_query_is_itself_fuzzy` — the reported case,
  `kmap validate` against `src/keymap/validate.rs`; the retained ordering rule;
  and that a term which had to spread out is still distinguishable from one
  that landed whole, which is what the secondary emphasis colour is for.
- `a_space_frees_the_distance_between_terms_but_not_inside_one` — the boundary
  scores better than the same distance inside a term, and costs exactly the one
  adjacency bonus it declines to pay.
- `inserting_into_the_query_can_only_narrow_what_matched` — splitting a term
  leaves the matched set unchanged, growing one from the middle narrows it, and
  appending still narrows from what is on hand.
- `directness_is_read_per_term_rather_than_from_the_run_count` — the
  cancelling case above, that adjacent terms are still direct, and that
  positions which cannot have come from the query claim nothing.
- `the_subsequence_filter_decides_exactly_what_the_scorer_decides` — unchanged,
  and now covers a single rule rather than two agreeing ones.

`docs/user-guide.md` states the new rule. `context/reference/fuzzy-matching.md`
records the benchmark movement: the `parser test` row went from 239 matched
candidates to 310 against fzf's 691, the remainder being the ordering rule. No
other row of the agreement table moved.

Known limitation: a fuzzy term can now be satisfied by characters scattered
through a basename, where the basename bonus is worth 30 per character and a
directory match is worth nothing. On this repository `src picker` consequently
ranks `src/app/tests/search_and_pickers.rs` first and `src/picker.rs` third,
where fzf puts `src/picker.rs` first. The weighting behind this is described
under "Why directories are in the corpus" in
`context/reference/fuzzy-matching.md`; rebalancing it is a design decision about
what a directory-segment match is worth against a basename match, and is not
part of this fix.

## Report

A query containing whitespace was split into terms, and each term then had to
occur in a candidate as a contiguous literal run. A query without whitespace
was matched as a fuzzy subsequence. Typing a space therefore replaced fuzzy
matching with exact matching for every term.

The terms were additionally required to appear in the order they were typed,
and not to overlap. That ordering rule is deliberate and was kept; only the
contiguity of each term was in question.

In the project finder's name mode, over this repository:

| Query | Result | Expected |
| --- | --- | --- |
| `kmap validate` | no results | `src/keymap/validate.rs` |
| `fpick fuzzy` | no results | `context/issues/resolved/file_picker_and_fuzzy_grep.md` |

`kmap` and `fpick` are abbreviations that no candidate contains as a contiguous
run, so the term matched nothing even though the intended candidate is
reachable by subsequence. Both terms in each query appear in the order typed,
so the ordering rule was not what rejected these candidates.

The same abbreviations worked as a single term. `fpick` alone finds
`src/file_picker.rs`, and `kmap` alone finds `src/keymap/`. The narrowing
attempt was what lost the match.

fzf returns the expected candidate for both queries.

Expected behavior: each space-separated term matched as an independent fuzzy
subsequence, with a candidate matching when every term matches it, in the order
the terms were typed. fzf does not have the ordering rule — `picker src` finds
`src/picker.rs` in fzf and should continue to find nothing here — so that part
is a deliberate divergence rather than an oversight. Whitespace continues to
separate rather than match, a single term continues to mean what it meant, and
smart case continues to read the whole query.

Constraints recorded with the report: `FuzzyMatcher::matches` decides exactly
what `matches_fuzzy` decides and exactly what `FuzzyMatcher::score` will
accept, so the scanner's filter, the picker's narrowing and the ranker had to
change together. The multi-term branch of `score` was built on the contiguity
being removed, including `last_term_start`, `term_start`, `term_at`,
`find_term`, and a hand-added `28 * (term.len() - 1)` that put one term and one
word on the same scale. Match positions stop being one run per term, which
`is_direct_match` and the preview highlighting read. The rule is shared by the
finder's name mode, the finder's content mode, `:fuzzy-grep`,
`:fuzzy-grep-directory`, and the commit picker on `Space g f`.

The report also noted that the exactness being removed was a narrowing tool
with nothing proposed to replace it. fzf reaches the same narrowing through a
separate syntax, where a term prefixed with `'` is matched exactly; Runyte has
no equivalent, and `^`, `$`, `!` and `|` are likewise fzf syntax that Runyte
reads literally. Whether an escape for an exact term is wanted was left as a
separate question and remains open.
