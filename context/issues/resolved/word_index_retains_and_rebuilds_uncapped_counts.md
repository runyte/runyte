---
title: "Completion retains uncapped word counts and rebuilds unchanged buffers"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 24a622a
---

## Resolution

Commit `24a622a` (`perf(completion): share capped word lists across index updates`).

`word_index::run` previously kept complete frequency maps for every indexed
buffer, then cloned and sorted every map whenever any buffer changed. It now
retains capped `Arc<BufferWords>` values and republishes shared references for
unchanged buffers. Word extraction streams rope characters into exact counts,
avoiding one allocated string per occurrence. Truncated vectors release their
excess capacity. Frequency and lexical ranking, eviction, removal, and older
snapshot contents retain their existing semantics.

`capped_index_keeps_frequent_words_after_the_distinct_word_limit`,
`snapshots_share_unchanged_word_lists_across_updates_and_removals`, and
`streaming_word_counts_preserve_unicode_hyphens_and_line_boundaries` in
`src/word_index/tests.rs` cover ranking, retained capacity, immutable sharing,
and Unicode token boundaries. All twelve word-index tests pass.

Known limitation: exact ranking temporarily counts every distinct word in the
one buffer being rebuilt; this change bounds retained index state, not that
necessary temporary counting map.

## Report

The word-completion worker retains the complete word-frequency map for each
indexed buffer. The 20,000-word per-buffer limit is applied only to a separate
published copy. Every subsequent update or removal clones and sorts the full
maps for all remaining buffers, including unchanged ones. Word extraction also
first allocates a string for every occurrence before counting duplicates.

Opening a document containing more than 20,000 distinct words reproduces the
retention issue. Typing in a different small buffer repeatedly clones and sorts
that large document's map. Repeated words additionally create temporary
allocations proportional to the number of occurrences rather than distinct
words. These costs run on a background thread, but increase memory use and
delay completion updates throughout the workspace.

The worker should retain only the capped ranked result for each buffer, share
unchanged results between snapshots, and count words as they are extracted.
Ranking must continue to use exact descending frequency with lexical ordering
for ties, including frequent words appearing after the first 20,000 distinct
tokens. Existing latest-update, removal, and buffer-eviction ordering must remain
unchanged. Computing exact frequencies may still require a temporary map of all
distinct words in the one buffer currently being rebuilt.
