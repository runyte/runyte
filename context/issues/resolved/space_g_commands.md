---
title: "The Space-g namespace contains too many Git commands"
status: resolved
reported: 2026-08-18
resolved: 2026-08-18
legacy_commit: 0157b67
---

## Resolution

Commit `0157b67` (`Simplify the Git key namespace`) fixed the registry in
`built_in_bindings`, which exposed Git mutations and secondary views globally
even though those operations belonged to the changed-file workflow. The
`Space g` namespace now contains only `g`, `d`, `r`, `t`, `f`, `l`, `b`, and
`B`. This also removes `u` and `w`, which were not repeated in the report's
move/remove list but were excluded by its explicit list of commands to retain;
their underlying `:git-unstage` and `:git-worktrees` commands remain available.

The registry-backed Git-status action menu now adds buffer-wide `Tab c` for a
commit message, `Tab i` for the staged review, and `Tab S` for staging all
outstanding rows. `Tab s`, `Tab u`, and `Tab D` remain row-scoped so a
selection continues to determine which files they change. A separate
`StageAllChangedFiles` command was added because reusing row-scoped
`git-stage` would have made `S` depend on the current selection instead of
staging the whole list. It collects each unique unstaged or untracked row and
then uses the same bounded synchronous or asynchronous staging path as the
existing selection command. The removed bindings do not remove their colon
commands or Git functionality.

Coverage is in `tests/keymap.rs` with
`git_namespace_keeps_only_navigation_and_refresh_commands`, and in `src/app.rs`
with `stage_all_action_stages_every_unstaged_row_not_just_the_selection` and
`every_key_the_changed_file_list_advertises_does_what_it_says`. The complete
change also passes `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`.

## Report

The `Space g …` namespace contained too many commands. In practice it was used
only for:

```text
Space g g -> then staging and commiting from there
Space g d
Space g r
Space g t
Space g f
Space g l
Space g b
Space g B
```

The requested changes were:

```text
Space g D -> should be available from Space g g in the Tab overlay
Space g S -> remove completely
Space g a -> remove completely
Space g c -> should be available from Space g g in the Tab overlay
Space g s -> should be available from Space g g in the Tab overlay
Space g i -> remove completely
```

The `Space g g` Tab overlay also needed an `S` command for staging all files.
