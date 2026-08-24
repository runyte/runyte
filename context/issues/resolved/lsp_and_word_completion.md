---
title: "Explicit LSP completion ignored the existing prefix and could fall back to word completion"
status: resolved
reported: 2026-08-19
resolved: 2026-08-19
legacy_commit: 5ee7ddb
---

## Resolution

Commit `5ee7ddb` (`Fix explicit LSP completion sessions`) fixed
`App::lsp_completion` and `App::after_insert`. The request anchor had been the
caret at the moment `Ctrl-x` was pressed, so the local filter contained only
characters typed after activation and ignored an existing prefix such as
`left_at`. If the next character matched no label from the unfiltered server
list, `after_insert` discarded the Language popup and immediately called
`word_completion`, which made the completion source appear to change
unpredictably.

Explicit completion now creates a generation-tagged Language session as soon
as its request is accepted. Its anchor is the beginning of the identifier
fragment before the caret, its filter is recomputed from live buffer text, and
Word and Path completion cannot replace it. A zero-match session stays active
without drawing an empty overlay, allowing Backspace to reveal cached matches;
`.` and `:` start a fresh generation for the new language context, and late
responses from cancelled or superseded generations are ignored. Space,
newline, dismissal, acceptance, caret movement, and unrelated editing commands
end the session, while character Backspace and Delete retain it.

`CompletionState::visible_indices` now filters Language candidates through the
server's `filterText` when supplied and orders them through `sortText`, using
the label as the protocol fallback for either field. When a completion has no
server-provided text edit, acceptance replaces the complete identifier prefix
rather than appending the candidate after text entered before `Ctrl-x`.

Coverage: `src/app.rs`
(`explicit_completion_filters_the_existing_prefix_and_replaces_it`,
`explicit_completion_stays_pinned_without_matches_and_rejects_late_responses`,
`explicit_completion_refreshes_context_and_ends_at_editing_boundaries`, and
`language_completion_uses_filter_text_and_sort_text`); and `src/lsp/mod.rs`
(`completion_preserves_server_filter_and_sort_text`). Run them with
`cargo test completion`.

Known limitation: Runyte still uses prefix filtering rather than fuzzy
matching, and does not yet act on LSP `preselect` or
`CompletionList.isIncomplete`.

## Report

Ordinary typing correctly opened word-completion suggestions. Pressing
`Ctrl-x` opened LSP completion, but the candidates were not filtered or ordered
from the text already typed. With `left_at` before the caret, `self::` appeared
first while `left_at` remained elsewhere in the overlay and had to be reached
with the arrow keys.

After `Ctrl-x`, typing another character could change the popup back to
standard word completion. This did not happen for every character, so the
trigger was unclear. Explicit LSP completion was expected to remain the active
source while typing until space began a new word, Enter began a new line, or
Escape cancelled completion.
