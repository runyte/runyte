---
title: ":split does not appear in the command hint window"
status: resolved
reported: 2026-07-30
resolved: 2026-07-31
legacy_commit: 63a3975
---

## Resolution

Fixed in commit `63a3975`, "Offer command hints under the spelling that
matched".

`App::matching_commands` in `src/app.rs` already matched aliases, so `:sp`
did find `hsplit` through its `split` alias — but it returned the
`CommandSpec`, and the hint window renders a spec under its canonical name.
The row therefore read `:hsplit`, and Tab completed to `hsplit`, replacing
what had been typed.

What changed:

- `matching_commands` now returns `CommandMatch` values, pairing each spec
  with the spelling that matched. A prefix that only an alias matches is
  listed under that alias, and the canonical name moves into the row's alias
  column.
- `CommandMatch::usage` retitles the usage line with the matched spelling, so
  the row reads `:split [path]` with `aliases: hsplit`.
- Tab completes to the spelling on screen rather than the canonical name, so
  it never rewrites a name someone deliberately typed.
- An empty query is still a table of contents: it lists every command once
  under its canonical name.

Covered by `command_hints_list_the_alias_that_matched` and
`command_hints_keep_canonical_names_when_they_match` in `src/app.rs`.

## Report

Typing `:sp` offered only `:hsplit` as a command hint, though `:split` ran
correctly and produced a horizontal split. `:split` should also appear in the
hint window.
