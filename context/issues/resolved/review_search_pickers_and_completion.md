---
title: "Word-index backpressure could lose final updates or resurrect closed buffers"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: b945b1c
---

## Resolution

Commit b945b1c (`Preserve final word-index actions`) replaced the word index's
separate lossy update queue and removal channel with one per-buffer
latest-action map and a single bounded wake slot. A burst now retains the
newest update for every distinct buffer, and a removal supersedes every older
update for that buffer, so delayed queue work cannot republish closed-buffer
words.

Each action receives a sequence while holding the pending-map mutex. Drained
actions are applied in sequence order, preserving the documented
least-recently-updated eviction policy across distinct buffers while still
coalescing repeated work for one buffer. Snapshot reads remain a short `Arc`
clone under a lock and never wait for indexing.

Coverage lives in `src/word_index.rs` in
`the_latest_update_survives_a_large_burst`,
`a_removal_supersedes_every_older_update`,
`same_batch_removal_precedes_capacity_replacement`, and
`same_batch_refresh_controls_the_next_capacity_eviction`, alongside
`worker_indexes_and_removes_buffers`. Completion integration remains covered
by `word_index_follows_buffer_open_edit_and_close` in
`src/app/tests/language.rs`.

## Report

Buffer search, project discovery, fuzzy matching, filterable pickers,
previews, path completion, word completion, and jump labels required a focused
hardening review. The scope included `src/finder.rs`, `src/file_picker.rs`,
`src/picker.rs`, `src/word_index.rs`, `src/jump_labels.rs`, search history and
picker workflows, completion support, and their tests.

The review covered cancellation and stale asynchronous results, result
ordering, selection stability, Unicode and case behavior, literal versus
regex behavior, zero-width matches, hidden and ignored files, symlinks,
binary and unreadable files, large repositories, preview bounds,
directory-listing cache invalidation, memory growth, and changes to the active
buffer while results are open. Existing Runyte search semantics were to remain
unchanged.
