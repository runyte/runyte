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
