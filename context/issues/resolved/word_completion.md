---
title: "Runyte had no completion sourced from words already open in the workspace"
status: resolved
reported: 2026-08-18
resolved: 2026-08-19
legacy_commit: 9c809c1
---

## Resolution

Commit `9c809c1` (`Add word completion from every open buffer`) added a third
completion source, `CompletionSource::Word`, alongside the existing
language-server and path sources. It reuses the existing `CompletionState`,
`handle_completion_key`, and `accept_completion` machinery unchanged — a word
candidate carries no server-supplied edit, so acceptance already took the
same "replace `anchor..head` with `item.insert`" branch that path completion
uses.

The index lives in a new module, `src/word_index.rs`: a background thread,
spawned the same way `git/service.rs`'s worker is, that owns a
`HashMap<usize, HashMap<String, u32>>` of per-buffer word counts and
publishes a read-only snapshot into a `Mutex<Arc<WordIndexSnapshot>>` after
every change. The main thread only ever reads that snapshot — a lock held
just long enough to clone the `Arc` — so a completion query can never block
on the worker, and updates arrive through a bounded, best-effort channel
where a dropped message is harmless because the next edit supersedes it.
Word extraction splits each line on Unicode whitespace, then trims a fixed
set of wrapper punctuation (backtick, quote, comma, semicolon, and matching
brackets) from both ends of each token without touching the interior, which
is what keeps `--workspace-restart`, `:quit-here`, and `background-color`
whole while still turning `` `--wls` `` into `--wls`.

`App::word_completion`, called from `after_insert`, triggers once the word
before the cursor reaches `editor.word_completion_minimum` characters,
building its candidate list from the buffer being typed in first (by
frequency, self-exclusion applied) and then every other buffer in the same
order, deduplicated by label. It never opens over an active Language or Path
popup, and a Language response is still free to replace it — `show_completion`
only ever protected Path. Buffer eligibility follows directly from
`Buffer::is_read_only`: every non-read-only buffer (`File`, `Directory`,
`Scratch`, `CommitMessage`) contributes words to the index, matching the
report's rule exactly, while the popup itself only opens in the three that
are being typed as prose (`File`, `Scratch`, `CommitMessage` — not the
explorer, whose text is a filename being renamed).

Keeping the index in sync needed three separate hooks rather than one,
because `App` has three distinct ways a buffer's text changes: transactional
edits reindex through `reconcile_applied_transaction`; undo, redo, file
reload, and a Git-triggered reload all replace a buffer's whole text through
the separate `resync_replaced_buffer` path and needed their own hook, added
after a review found that without it a reverted word could remain offered as
a candidate indefinitely; and a newly opened buffer is picked up by a
per-frame sweep in `prepare_view` rather than at each of the dozen
`self.buffers.push` call sites, relying on buffer ids being append-only and
never reused. Closing a buffer removes its words via `close_buffer`, and that
removal travels on its own unbounded channel rather than the lossy update
one — a review found that under backpressure a dropped `RemoveBuffer` had no
later message to supersede it the way a dropped update always does, so it
could leave a closed buffer's words in the snapshot forever.

`editor.word_completion` (default on) and `editor.word_completion_minimum`
(default 3) were added to `EditorConfig` and to the `Space o o` settings
registry, following the existing boolean and integer descriptor templates.

Two follow-up commits changed accept-key behavior in response to using the
feature. `9f93540` stopped Enter from accepting a word completion, because a
word popup opens for any three-character prefix match — far more often than
a trigger character ever caused a Language or Path popup — which made
finishing a line of prose with Enter a lottery between a newline and an
unwanted word; Tab still accepted. `d76dc2d` then extended that to every
completion source for consistency, since a Language or Path popup can
likewise open without being asked for: Enter is now reserved for its usual
newline everywhere, and only Tab accepts, with the popup titling itself
"LSP Complete" rather than the generic "Complete" for a language-server
response.

Coverage: `src/word_index.rs` (`preserves_examples_from_the_issue`,
`trims_surrounding_punctuation_only`, `discards_pure_punctuation_tokens`,
`keeps_interior_punctuation_untouched`, `splits_on_whitespace_across_lines`,
`worker_indexes_and_removes_buffers`, `a_removal_survives_a_saturated_update_queue`);
`src/buffer.rs` (`word_completion_eligibility_matches_read_only_and_directory_status`,
exhaustive over every `BufferKind`); `src/app.rs`
(`word_completion_triggers_after_the_minimum_and_orders_own_buffer_first`,
`word_completion_is_replaced_by_a_language_response_but_never_opens_over_one`,
`word_completion_yields_to_a_typed_path`,
`word_index_follows_buffer_open_edit_and_close`,
`word_index_resyncs_after_undo`,
`word_completion_queries_skip_a_typed_opening_wrapper`,
`word_completion_lets_enter_insert_a_newline_instead_of_accepting`); and
`tests/local_protocol.rs`
(`git_commit_wait_tui_completes_through_write_quit`, updated to send Escape
twice, since a word popup can legitimately be open after typing an ordinary
commit message whose own boilerplate text shares words with it).

Known limitation: the explorer's directory listing can also refresh outside
the transaction path, when the filesystem changes underneath it; that route
does not push an index update, so a renamed or newly created sibling file
becomes a candidate only once some other edit or newly opened buffer triggers
the next sync, not immediately. The per-buffer and per-workspace word counts
the index retains (20,000 words per buffer, 256 buffers) are fixed
backstops, not configurable settings, in the same spirit as the existing
notification history limits.

## Report

Runyte should complete words from the text already open in the workspace,
with the same interaction path completion has today: candidates appear on
their own while typing, without a key being pressed to ask for them.

The candidates come from the words in every open buffer of the workspace,
including the explorer buffer, whose text is filenames — completing a name
that echoes a sibling file is part of the point. No other special buffer
contributes: the Git views, notifications, help, the config view, and the
about page hold generated text that nobody is trying to retype.

A word is a run of characters between whitespace. That is deliberately wider
than an identifier, because the text this repository is written in is full
of things an identifier rule would split into uselessness —
`--workspace-restart`, `:quit-here`, `background-color` — and completing one
of those in a single acceptance is worth more than a tidier index.
Punctuation that merely surrounds a word rather than belonging to it, such as
the backticks around `` `--wls` `` or a trailing comma, should not end up
inside a candidate; exactly how much to trim is an implementation decision.

Candidates appear once the typed prefix reaches a configured number of
characters, three by default. Both the trigger length and whether the
feature runs at all belong in `Space o o`, since automatic completion is
exactly the kind of thing that some people find helpful and others find
distracting. Suggested spellings are `editor.word_completion` and
`editor.word_completion_minimum`.

Candidates from the buffer being typed in come first, ordered by how often
each occurs in it, and words from other buffers follow ordered the same way.
The word currently being typed is never offered as a completion of itself.

Completion appears in file buffers, in the scratch buffer, and in the commit
message. The commit message is generated but is real writing, and completing
an identifier from the change being described is worth having there. It
stays silent in the explorer, where the text is a filename being renamed,
and in every read-only buffer.

This must not be felt while typing. The index belongs to a worker thread
that receives buffer changes and answers queries from a snapshot, so a
keystroke never waits on it and a result that is one keystroke out of date
is acceptable. Memory needs a bound, in the same spirit as the existing
per-entry and per-workspace notification limits.

These completions are Runyte's own and are separate from what a language
server offers. `Ctrl-x` continues to request language-server completions
explicitly, and wins: pressing it while word completions are showing
replaces them with the server's answer. Word completions never replace an
open language-server completion. Path completion is unaffected, because it
triggers on `/` rather than on a prefix length, and continues to take
precedence when a path is what is being typed.
