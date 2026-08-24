---
title: ":close needs a short alias, and Space q quits too easily"
status: resolved
reported: 2026-07-30
resolved: 2026-07-31
legacy_commit: 85e0d37
---

## Resolution

Fixed in commit `85e0d37`, "Make quitting typed and give :close a short
alias".

What changed:

- `close` gained the `c` alias in the `COMMANDS` table in `src/app.rs`, so
  `:c` closes the active pane.
- The `Space q` and `Ctrl-q` bindings were removed from `src/keymap.rs`.
  `Ctrl-q` went along with `Space q` because the report asks to keep *only*
  `:quit[!]` and `:q[!]`, and a Normal- and Insert-mode chord is the same
  accident waiting to happen. With no binding left, the `EditorCommand::Quit`
  identity was unreachable, so it was removed from `src/command.rs` too.
- `close_pane` already refuses to close the last pane, so no key sequence can
  end the session any more.

Documented in `README.md` and `context/reference/helix-keymap-v1.md`, where
the `Space q` row is now marked **Changed**.

Covered by `closing_a_pane_has_a_short_alias` and
`leaving_the_editor_is_typed_rather_than_bound` in `src/app.rs`.

## Report

Closing a pane required typing `:close` in full. Two changes were requested:

1. Add a `:c` alias.
2. Remove the `Space q` shortcut, which exits the whole program too easily.
   Quitting should require `:quit[!]` or `:q[!]`.
