---
title: "Pane boundaries cannot be resized with colon commands"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: 4a6edff
---

## Resolution

Commit 4a6edff (`Add directional pane resize commands`) added the typed
`resize-right`, `resize-left`, `resize-top`, and `resize-bottom` colon
commands. `pane_neighbor` identifies the adjacent pane sharing the named
edge, and `resize_pane_edge` reuses `Layout::resize_between_cells`, the same
cell-exact, minimum-size-aware operation used by mouse resizing. The parser
accepts either spaced or compact signs, such as `+ 4`, `- 2`, `+4`, and `-2`.

Runyte deliberately measures `N` in terminal cells rather than pixels because
the terminal frontend exposes a character-cell grid and its layout has no
pixel coordinate model.

Coverage is in `pane_resize_commands_parse_signed_cell_counts` in
`src/command.rs` and `resize_commands_move_each_named_boundary_in_cells` in
`src/app.rs`. `command_mode_renders_the_filterable_command_palette` in
`src/ui.rs` filters to its intended syntax row so the expanded command
inventory does not make that presentation check depend on viewport ordering.

## Report

Commands were requested for pane resizing:

```
:resize-right +/- N
:resize-left +/- N
:resize-top +/- N
:resize-bottom +/- N
```

`N` is a count of pixels. `+` moves the named boundary so the active pane
grows; `-` moves it so the active pane shrinks.
