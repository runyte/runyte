---
title: "Space g w no longer opens the worktree list"
status: resolved
reported: 2026-08-19
resolved: 2026-08-19
legacy_commit: 6ff9195
---

## Resolution

Commit `6ff9195` (`Add back Space g w for :git-worktrees`) restored the
`Space g w` binding to `ColonCommand::GitWorktrees` in `src/keymap.rs`. It had
been removed by an earlier commit, `0157b67` ("Simplify the Git key
namespace"), which trimmed the `Space g` namespace down to navigation and
refresh commands and left `:git-worktrees` reachable only by typing the colon
command. That earlier removal was a deliberate simplification at the time,
but going through the command palette for a routine navigation action
regressed convenience the namespace otherwise provides for every other Git
view (`Space g b/g/l/f/B/d/r/t`), so the binding is added back alongside
those.

`README.md`'s Git key table and worktree section, and
`context/reference/helix-keymap-v1.md`'s `Space g` namespace row, were
updated to list `w` among the retained keys.

Coverage lives in `tests/keymap.rs`: `" gw"` was added to the retained-case
table in `nested_space_tree_is_exact_primary_and_keeps_fast_compatibility_paths`,
and moved from the removed list to the retained list in
`git_namespace_keeps_only_navigation_and_refresh_commands`. The keymap
inventory counts asserted in
`app::tests::command_inventory_classifies_every_command_and_current_binding`
(`src/app.rs`) were updated for the added binding.

## Report

`Space g w` should open `:git-worktrees` again.
