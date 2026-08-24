---
title: "Insert-mode path hints only reopened when the typed character was /"
status: resolved
reported: 2026-08-19
resolved: 2026-08-19
legacy_commit: b8401f6
---

## Resolution

Commit `b8401f6` (`Reopen path hints on any keystroke, not only typing /`)
fixed `App::path_completion` and `App::after_insert` in `src/app.rs`.
`path_completion` required the token before the caret (`path_token_before`)
to end with `/` before it would list a directory at all, so the popup could
only ever be *opened* at the instant a trailing `/` had just been typed.
`after_insert` reinforced this by only calling `self.path_completion()` when
the just-typed character was `/`; once the popup was open, further
characters already grew its filter locally, but nothing could reopen it once
closed except retyping a `/`.

`path_completion` now splits the token on its last `/` instead of requiring
one at the end, listing the same directory — resolved against the active
file's parent and the project root exactly as before — while seeding the
popup's `anchor` and `filter` with whatever fragment already follows that
`/`, the same pattern `word_completion` already used for its own anchor and
filter. `after_insert` now calls `path_completion()` whenever no popup is
currently open, not only when the character is `/`; `/` itself is kept as an
unconditional trigger so it still overrides an in-progress Word popup exactly
as it did before. A `was_path_completion || self.path_completion_active()`
check after that call skips the following word/LSP-trigger-character
handling on the same keystroke, whether the popup was already open or just
opened by it.

The colon-command path hints (`App::matching_path_hints`, used by `:open`
etc.) are a separate mechanism that was already not gated on `/`; they were
unaffected by this change.

Tests covering the behavior are in `src/app.rs`:

- `path_completion_reopens_while_editing_an_existing_path_without_retyping_slash`

Known limitation: Backspace and Delete still dismiss the popup like any
other editor command, as before this fix — only the set of characters that
can *open* it was widened, not what keeps it open.

## Report

Path hints opened only when slash (`/`) was typed. Editing an existing path
after the popup had closed therefore required deleting the path fragment back
to its last slash and typing the slash again. A path token should reopen hints
while it is edited, for both absolute paths and paths resolved relative to the
active file or project root.
