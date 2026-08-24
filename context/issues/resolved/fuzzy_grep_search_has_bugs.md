---
title: "Fuzzy grep cannot find matches outside the part of a project its scan read first"
status: resolved
reported: 2026-08-19
resolved: 2026-08-19
legacy_commit: 7a71570
---

## Resolution

Commit `7a71570` (`Search a project in the content scan, not after it`)
resolves the issue.

`scan_content` collected every non-empty line below the picker root as a
ranking candidate and stopped at `CONTENT_ENTRY_LIMIT`, then 10,000 of them.
The query was applied afterwards, by `FilePicker::rank`, over whatever that
walk had gathered. So the ceiling bounded how far into a project the scan got
rather than how many results it found, and in any project larger than that
budget the candidate list was simply the first files the walk reached. This
repository is 147,000 lines and `src/app.rs` alone is 37,000, so a single file
could consume the whole budget. A match outside that prefix could not be typed
into view at all. Opening its file made it appear because `open_picker_at`
injects every open buffer's live text as candidates, which is exactly the
workaround the report describes.

The scan now carries the query. The line filter keeps only the lines that query
matches, so the ceiling bounds matches and `limited` means the
project holds more than the budget of them — a state a longer query resolves,
rather than a silent horizon. The filter runs on the truncated line text the
picker will rank, not on the whole line, so a query matched against the tail of
a very long line cannot produce an entry nothing can highlight.

Because a scan now belongs to one query, editing the query has to restart it.
`FilePicker` records the `scan_query` its entries were collected for;
`content_rescan_needed` decides when they can no longer answer, and
`App::start_content_scan` cancels the running scan and begins a new one under a
fresh id, re-collecting open-buffer text under the new query. Every path that
edits a content query funnels through `restart_content_scan_if_needed`: the
picker key handler, bracketed paste, and the arrival of a `Finished` event that
reports truncation.

Not every edit rescans. A scan that completed without truncation holds every
line matching its query, and appending characters can only narrow that set, so
those entries stay authoritative and the narrowing happens in memory. Only a
query that is no longer an extension of the scanned one, or one whose scan was
truncated, walks the project again. That is why a truncated `Finished` triggers
a restart: typing on was allowed to narrow in memory while the scan was still
running, and learning only at the end that it stopped early is the one case
where that narrowing would have lost results.

The filter itself is `matches_fuzzy`, an allocation-free pass over a line. It
decides exactly what `fuzzy_match` decides — the scorer's dynamic programming
reaches a final state precisely when the query occurs as an ordered
subsequence — so the scan can neither hide a line the scorer would have ranked
nor collect one it cannot rank. `fuzzy_match` takes it as an early exit, and
the smart-case character comparison the two share gained an ASCII branch,
because `char::to_lowercase` builds an iterator per call and this now runs once
per character of every line in a project.

The background worker waits out a short, cancellable settle window before it
reads anything, so a fast typist starts one project read rather than one per
character; the intermediate scans are cancelled while still asleep.

Deliberate deviations from the report: the request was to find bugs in the
existing design, and the diagnosis is that the ceiling was in the wrong place
rather than that any single step was wrong. Raising the limit was rejected —
2,000,000 candidates is roughly 300MB held and far past what can be re-ranked
inside a frame — so the limit moved from the corpus to the results instead.

A follow-up raised `CONTENT_ENTRY_LIMIT` from 10,000 to 50,000 after asking
what the number was for. Nothing had derived it: commit `0114dd5` introduced it
bare, a round figure for keeping filtering interactive. What binds it is the
rank pass a keystroke pays for, which is `O(candidates × query × line)` and
measurably linear in the count, so the follow-up bought headroom before
spending it. `FuzzyMatcher` prepares a query once and reuses its
dynamic-programming table, score rows, prefix row and character vector across
candidates, replacing roughly `3 × query + 9` allocations per candidate with
one; `rank_entries` then divides the pass across cores above
`PARALLEL_RANK_CHUNK`, joining chunks in candidate order so the sequence handed
to the sort is the one a single thread would have produced. Together those are
about 2.5×, which is what the budget went up by at unchanged worst-case
latency: on this repository the slowest keystroke at 50,000 candidates is about
24ms, against 17ms at 10,000 before the follow-up, and every query past a
single character now arrives complete rather than truncated. 100,000 was
measured and rejected at 50ms a keystroke.

A second follow-up took the path heuristics out of content ranking. Two of the
scorer's rules are about paths: the characters after the last `/` are the
basename and are worth 30 more each, and a candidate loses 3 a separator so a
shallow path outranks a deep one. Both were being applied to line text, where
they misfire. One search over this repository showed it: eighteen commit lines
in one file hold the same exact match, seventeen scored 401 to 408, and the one
ending `(origin/main, origin/HEAD)` scored 220 — the basename start had moved
past the match, costing it 30 a character — which put it at rank 717 instead of
beside its siblings. `FuzzyCandidate` now says whether a candidate
is a path or a line, `FilePicker` derives it from its own kind, and a line
takes neither rule. The base score still rewards a line that equals or begins
with the query, because that is meaningful for text.

A third follow-up made a space separate the query into terms instead of being a
character to match. The scan and the ranker both read a query as one ordered
subsequence, so `content entries` returned 272 lines of which one held the two
words contiguously, and `pub fn score` returned 205 of which two did. Ranking
put the real matches on top, so nothing was lost, but the tail below them was
noise and the count said nothing. Requiring each term as a *fuzzy* term was
measured to change nothing at all — if the whole query is a subsequence then so
is each of its terms — so a term is required as itself.

One word keeps the fuzzy subsequence it has always been, and its scoring path
is untouched. Two or more are matched literally and in order, which is what
keeps the rest of the picker sound: positions stay sorted, the jump column
stays the first matched character, and a longer query still only narrows what a
shorter one matched, so `content_rescan_needed` may go on reusing the entries
on hand. Because terms are contiguous there is no alignment to search, so
`score` does not run the dynamic programming for them; it picks, for each term
in order, the occurrence scoring highest under the same per-character rules,
bounded by how far right that term may sit while leaving room for the ones
after it. The measured effect on this repository: `content entries` 272 to 18,
`pub fn score` 205 to 2, `fn rank entries` 68 to 3. The rule applies to the
file picker too, where `src picker` now finds `src/picker.rs` and one other
file rather than everything spelling those letters in order.

Review of that change found the narrowing it relies on had become unsound in
one direction. `FilePicker::rank` may look only at the previous result set when
the new query cannot match anything the old one missed, and under a subsequence
every edit that adds characters had that property. Under terms only growing the
query at its end does. Typing into the middle rewrites a term rather than
extending it, and a term matched literally can widen when it changes: `ab cd`
becoming `a b cd` starts matching `a_x_b_cd`, which the whole term `ab` had
thrown out and which narrowing would never look at again. This is not a
question of whitespace — `ab cd` becoming `aXb cd` widens the same way without
touching a term boundary — so the guard is where the caret was, not what was
typed. `insert_query` and `insert_query_text` now narrow only when the
insertion landed at the end of the query, which is the same condition
`content_rescan_needed` uses, and re-rank everything otherwise.

`is_direct_match` had to learn the same vocabulary, since a several-word match
is never contiguous and would otherwise have been painted as the gapped
subsequence it is not. It now asks whether the positions form no more runs than
the query has terms. Fewer is not a weaker match but a stronger one: terms that
happen to sit next to each other merge into one run, which is the tightest a
several-word query can land. The query is already in scope wherever the colour
is chosen, so this cost no protocol change.

A fourth follow-up stopped giving every matching line its own copy of its
file. Matches cluster: at a full budget on this repository the 50,000
candidates come from 207 files, and holding a `PathBuf` and a rendered
`relative` per line spent 2,384,332 bytes on paths where the distinct paths are
13,540 — as much memory as all the matching text — while rebuilding `relative`
per line through `strip_prefix` and a component walk cost 15.9ms on every
batch.

The scanner now groups its results, emitting a `FileHits` per file rather than
a `ContentEntry` per line, which is the natural shape: it already knows which
file it is reading, so nothing downstream has to rediscover it. `FilePicker`
keeps a table of the distinct files and a `FileEntry` holds a `u32` into it, so
a line costs its own text and four bytes. Anything outside the picker reads an
entry through `EntryView`, which resolves the file and carries `label` and
`match_positions_in_label`, so the call sites in `app.rs` and `ui.rs` kept
their shape. The candidate budget now counts lines across groups rather than
groups, which the existing scan test caught immediately when it did not.

Measured after: 2,384,332 bytes of path became 13,540, and `add_content` went
from 15.9ms to 10.1ms. What remains in it is the ranking and sort of 50,000
fresh entries, not path building. The private workspace protocol was unaffected
— it serializes the flattened overlay rows, never a `FileEntry` — so there was
no version to bump. `FileEntry` and `ContentEntry` were public, so this is a
breaking change to the library API, which `0.0.x` allows.

Tests covering the behavior live in `src/file_picker.rs`:
`the_subsequence_filter_decides_exactly_what_the_scorer_decides`,
`a_content_scan_reaches_a_match_far_past_the_candidate_limit`,
`content_entries_are_narrowed_in_memory_only_while_the_scan_was_complete`,
`a_restarted_content_scan_replaces_the_one_it_cancelled`,
`a_slash_later_in_a_line_does_not_decide_where_a_match_ranks`,
`a_query_of_several_words_asks_for_each_of_them`,
`several_words_score_where_each_of_them_landed`,
`a_longer_query_can_only_narrow_what_a_shorter_one_matched`,
`splitting_a_term_reconsiders_paths_the_whole_term_excluded`,
`a_file_is_held_once_however_many_of_its_lines_match`, and
`the_candidate_budget_counts_lines_rather_than_files`;
`fuzzy_grep_reaches_a_match_the_candidate_limit_used_to_hide` and
`fuzzy_grep_searches_contents_at_both_roots_and_enter_jumps_to_the_match` in
`src/app.rs`; and, in `tests/performance.rs`,
`a_content_scan_finds_a_match_anywhere_in_a_large_project` and
`ranking_a_full_content_budget_stays_within_a_frame`, which build a 600-file,
360,000-line project once and reuse it. On that fixture an optimized build
scans the whole project for one match in about 48ms; the budgets are set well
above what it measures, so they catch a pathological change rather than a few
percent. The two fixtures that have to sit on the far side of the candidate
budget are sized from `CONTENT_ENTRY_LIMIT` rather than from a fixed number, so
raising it again cannot quietly turn them into tests that would pass without
the fix.

Known limitation: a query of one word stays a fuzzy subsequence, so it matches
far more lines than contain it. One six-character search over this repository
returns 803 lines of which 25 hold the string; ranking puts all 25 first, in
order, but the tail below them is noise. A second word is what narrows it, and
there is no way to ask for one word literally.

The ceiling still exists, and a query matching more than
50,000 lines reports `result limit reached` and shows only the first 50,000
found, which are the first the walk reached rather than the best scoring.
Content search still skips files over 4 MiB and files that are not valid
UTF-8, and still cannot match across a line boundary. In the synchronous
path used by embedders without a background scanner, a keystroke that needs a
rescan blocks for the length of that scan; the terminal editor always attaches
the scanner and is unaffected.

## Report

In large projects `Space s g` does not always find all matches. Sometimes a
file where a match is known to exist has to be opened first, and only then does
`Space s g` find it.

The fuzzy grep search code should be analyzed for the bugs responsible for this
behavior, and performance tests added to establish that the feature is fast and
accurate on large projects with many files and large files.
