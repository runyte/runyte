---
title: "Directory tree lacks modal navigation, hidden-file toggling, search, and focused close behavior"
status: resolved
reported: 2026-09-29
resolved: 2026-09-29
commit: 07661b7
---

## Resolution

Commit `07661b7` — `Complete directory tree navigation and close behavior` —
adds the missing tree actions and routes close commands to the focused sidebar.

`App::handle_directory_tree_command` consumed inherited document commands
without applying them to the tree. Explicit directory-tree bindings now route
`gg` to the root, `ge`/`G` to the last expanded row, `Ctrl-b/f` to full pages,
`Ctrl-u/d` to half pages, and `gt/gc/gb` or `H/M/L` to viewport positions.
The keymap remains the source for execution, contextual help, hints, and the
remappable legend. Digits retain their existing destination-pane actions.

`read_directory` previously discarded dotfiles before caching the listing.
The bounded cache now retains them and `DirectoryTree::append_rows` applies
visibility when producing rows. The tree-local `.` override therefore applies
immediately to cached collapsed directories and outstanding listing results.
Hiding a selected hidden path selects its nearest visible ancestor; toggling
also clears explicit-reveal visibility exceptions. The configured starting
value and explorer settings remain separate from the tree override.

`/` now opens a dedicated `DirectoryTreeSearch` prompt and searches visible
entry names using a case-insensitive regular expression. `(?-i)` opts into
case-sensitive matching. Enter selects the next match, empty input repeats the
last tree query, and Escape cancels without moving. `Ctrl-n/p` cycle with
wrapping while `n` retains its create action. Invalid expressions preserve
the last valid query. Tree search leaves buffer text, selections, and the
buffer search query untouched. Bundled protocol version 68 carries the new
prompt kind to attached clients.

Colon `quit` and `close` previously bypassed tree command dispatch and reached
the backing pane or buffer; `window-close` was consumed without closing the
tree. Their focused-tree paths now call `App::hide_directory_tree`, preserving
buffers, panes, unsaved text, and terminal sessions and restoring the previous
pane's mode. Aliases `:q`/`:wc`, long spellings, and `:q!`/`:close!` agree.
When the tree is unfocused, ordinary close behavior remains in effect; `:qa`
still requests workspace-wide exit.

Validation on Linux: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
and `cargo test` pass (4,213 passed, 41 ignored). The canonical
`cargo llvm-cov --locked --workspace` run passes at 92.03% line coverage, above
the unchanged 89% floor. Native macOS and Windows validation remains CI-owned.

Regression coverage in `src/app/tests/directory_tree.rs`:

- `tree_modal_motions_use_tree_rows_and_viewport` covers physical key dispatch,
  boundaries, full and half pages, and viewport positions.
- `tree_dot_toggle_handles_pending_and_cached_listings_and_hidden_ancestors`
  covers outstanding listings, cached collapsed directories, hidden selected
  ancestors, explicit reveal exceptions, and hiding/reopening the sidebar.
- `tree_search_wraps_visible_names_without_searching_or_editing_the_pane`
  covers Unicode names, regex matching, wrapping, cancellation, hidden and
  collapsed entries, prompt snapshot conversion, and buffer isolation.
- `tree_empty_search_and_invalid_regex_preserve_the_last_query` covers empty
  input and malformed-pattern recovery.
- `tree_close_commands_hide_only_the_focused_tree` covers typed close commands
  and aliases over unsaved text with one or multiple ordinary panes.
- `tree_close_preserves_the_backing_terminal_and_unfocused_close_targets_the_pane`
  covers terminal retention and closing an ordinary pane beside an unfocused tree.
- `tree_cached_legend_and_help_follow_remapped_keys_and_resize` now also covers
  remapped hidden-file, search, and root-motion spellings and search dispatch.

`command_inventory_classifies_every_command_and_current_binding` in
`src/app/tests/commands.rs` records the additional scoped bindings.

Known limitation: tree search visits names in currently expanded visible rows;
it does not recursively scan collapsed directories or filter the tree. The
report left this choice unspecified. Numeric motion counts remain unavailable
because digits select destination panes.

## Report

The directory tree introduced for [GitHub issue #6](https://github.com/runyte/runyte/issues/6)
lacks several navigation actions available elsewhere in the editor:

- `gg`/`ge`/`G` and related page and viewport motions do not navigate tree rows.
- `.` does not show or hide hidden files.
- `/` does not search the tree.

With the tree focused through `Space d d`, those keys should operate on the
tree rather than the ordinary pane behind it. The report does not specify
whether search should traverse collapsed directories or filter visible rows,
or which keys should repeat a search given that `n` creates an entry.

Additionally, `:q`, `:wc`, and `:close` should close the focused directory tree
and return focus to the previous pane. They must preserve the other panes,
open buffers, unsaved text, and terminal sessions. The existing tree retains
its navigation state when hidden, and Escape returns focus without hiding it.
