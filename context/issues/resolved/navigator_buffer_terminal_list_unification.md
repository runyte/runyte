---
title: "Navigator, buffer picker, and terminal list use three different designs"
status: resolved
reported: 2026-09-26
resolved: 2026-09-26
commit: 47e846a
---

## Resolution

Commit `47e846a` (`Unify Navigator, buffer and terminal destination lists`)
resolves the issue. `open_buffer_picker` and `open_terminal_list` constructed
independent rows and actions, while `open_navigator` owned a third layout,
fuzzy matching, recent-activation ordering and visit behavior. Their separate
builders let the presentation and activation rules diverge for the same open
destinations.

All three entry points now use `open_destination_list` in
`src/app/navigation_workflows.rs`, scoped to all destinations, buffers or
terminals. The common row builder separates visibility, kind, name and state;
refreshes metadata without changing opening order or action identity; and
keeps every applicable buffer or terminal flag. The terminal scope retains
exited screens after running terminals. Enter uses `visit_destination` to
focus an existing pane, while Tab's explicit Bring into active pane action
retains the ability to move a terminal's single view. Close hidden buffers and
Close all exited terminals retain their workspace-wide scope.

The list keymap scope introduced by the session-manager fix supplies the
shared pinned legend. All destination scopes enable the bounded preview on
opening and use the Navigator's fuzzy resource fields, including terminal
IDs. Terminal program and directory details move into the preview; terminal
pane titles expose the ID accepted by typed commands.

Semantic row tints carry kind and state through snapshots, and five theme
keys distinguish destination groups in every bundled theme. Both renderers
shorten overlong names in the middle while preserving the trailing STATE
column. Elision uses character offsets after cell-based column padding, and
recognizes native Windows path separators. Protocol version 61 gates the new
required theme fields and row presentation metadata, preventing older bundled
clients and hosts from accepting incompatible frames. The stable plugin
contract is unchanged.

The subagent review found the missing protocol bump, Windows separator
handling and a character/cell offset mismatch for Unicode TYPE labels. All
three were fixed and rechecked. Linux validation passed `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test` and the canonical
`cargo llvm-cov --locked --workspace`, with 92.04% total line
coverage against the unchanged 89% floor. Socket and subprocess fixtures were
run outside the execution sandbox after a sandboxed run encountered permission
errors. Native macOS and Windows checks remain for CI.

Regression coverage:

- `every_destination_list_orders_by_recent_activation_and_names_its_keys`,
  `the_buffer_list_visits_a_buffer_where_a_pane_already_shows_it`,
  `the_terminal_list_shows_an_exited_terminal_and_finds_terminals_by_id`,
  `navigator_aligns_types_titles_and_flags_in_terminal_cells`, and
  `destination_elision_starts_at_the_name_after_a_unicode_type` in
  `src/app/tests/session_navigation.rs` cover scope, ordering, key discovery,
  visit semantics, terminal IDs and column offsets.
- `navigator_save_refreshes_metadata_and_preserves_order_actions_and_selection`
  and `navigator_selected_terminal_exit_requires_a_new_selection_before_accepting`
  in the same file preserve metadata refresh and stale-selection protection.
- `buffer_picker_uses_names_and_project_relative_or_absolute_paths` in
  `src/app/tests/editing_and_buffers.rs` checks modified state and semantic
  tint; `the_terminal_list_describes_each_session_and_says_so_when_there_are_none`
  in `src/app/tests/commands.rs` checks terminal visibility, state and preview.
- `an_overlong_destination_name_keeps_its_file_name`,
  `an_overlong_windows_destination_keeps_its_prefix_and_file_name` (Windows),
  and `destination_rows_colour_their_type_and_state_in_both_renderers` in
  `src/ui.rs` cover shortening and local/attached drawing.
- `bundled_themes_give_each_destination_kind_its_own_colour` in `src/config.rs`
  checks the bundled palettes; `overlay_row_availability_survives_the_wire_round_trip`
  in `src/protocol/frame.rs` now includes tint and elision metadata.
- `visiting_a_visible_terminal_focuses_it_and_bring_here_moves_its_single_view`
  and `the_pane_is_named_by_the_title_the_child_sets` in `tests/terminal.rs`
  cover pane focus, explicit movement, stable PTY geometry and terminal titles.
- `hidden_terminal_output_while_detached_is_unread_after_reattach` in
  `tests/persistent_host.rs` verifies the STATE column across a real host
  detach/reattach.

## Report

The Navigator (`Space n`), the buffer picker (`Space b b`) and the terminal
list (`Space t t`) all list open destinations, but each one lays out rows,
marks state, orders entries, filters, and names Enter differently:

| | Navigator | Buffers | Terminals |
|---|---|---|---|
| Row shape | aligned columns: kind · name · flags · `pane N` | `name [flags]` and a dimmed relative path | `name` and `#id · state · dir · shown · unread` |
| Visible-in-a-pane marker | `pane N` | `*name*`, active buffer only | `shown` |
| Enter | `visit` | `open` | `show` |
| Preview default | off | on | on |
| Order | recent activation | buffer creation | running first, then by ID |
| Filter | fuzzy over name and path fields | `ListPicker::new` | `ListPicker::new` |

Panes are not numbered on screen, so `pane N` identifies nothing a user can
see. The terminal rows repeat the launch program (`bash`) and, for terminals
with a default name, the directory already contained in the name.

None of the three overlays mentions the generic list chords it accepts, which
are matched directly in `handle_list_key` in `src/app/language_workflows.rs`
rather than through the keymap registry:

- `Ctrl-n` / `Ctrl-p` (and `Down` / `Up`, `Shift-Tab`) move the selection.
- `Ctrl-d` / `Ctrl-u` (and `PageDown` / `PageUp`) move by a page.
- `Home` / `End` jump to the first or last row.
- `Ctrl-t` toggles the preview.
- `Delete` clears the filter; `Backspace` removes one character.
- `Ctrl-c` closes the overlay, as `Esc` does.

## Expected behavior

### One list, three scopes

The three overlays are one list component with one row layout. `Space n` shows
every open destination; `Space b b` shows its buffer rows; `Space t t` shows
its terminal rows. `Space t t` continues to list exited terminals, dimmed; the
Navigator's rule for which terminals it includes is unchanged.

### Columns

```text
Navigator — runyte-dev · Enter visit · Tab actions · Esc close
> type to filter
    TYPE        NAME                                          STATE
  * [scratch]   scratch                                       [+]
  * [file]      README.md                                     [+]
  * [file]      docs/user-guide.md
    [about]     about                                         [RO]
    [config]    ~/.config/runyte/config.toml                  [RO]
    [file]      /tmp/prompt-8e155cfe-06b3…90d4.md             [STALE]
    [explorer]  .
  * [terminal]  ✳ Article usage in mode descriptions
    [terminal]  user@host:~/code/runyte-dev                   exited

  Ctrl-n/Ctrl-p move · Ctrl-d/Ctrl-u page · Home/End first/last
  Ctrl-t preview · Delete clear filter
```

- **Column 0** holds `*` when the destination is shown in any pane, and is
  blank otherwise. It replaces `pane N`, `shown`, and the `*name*` wrapping.
- **TYPE** holds the bracketed kind used in pane titles: `[file]`,
  `[explorer]`, `[scratch]`, `[about]`, `[config]`, `[help]`, `[terminal]`.
- **NAME** holds a file's path relative to the workspace root, a path outside
  the workspace with `~` for the home directory, or the terminal's name. A path
  too long for the column is truncated in the middle, keeping the file name.
- **STATE** holds every flag that applies, in one place: `[+]`, `[STALE]`,
  `[RO]` for buffers, and `exited`, `unread`, `bell` for terminals. `[+]` and
  `[STALE]` can apply together and are both shown.
- A dim header row names the columns.
- The trailing `bash · pane N` detail is removed.

The launch program and working directory of a terminal remain available in the
preview.

### Colors

The TYPE cell is colored by kind group: files, explorer, generated read-only
buffers (`about`, `config`, `help`), scratch, and terminals. Only the TYPE cell
is colored, so the selection highlight and the dimming of exited terminals keep
working. STATE flags use semantic colors: `[+]` modified, `[STALE]` warning,
`[RO]` dim. Colors come from theme keys with values in both light and dark
bundled themes.

### Behavior

- **Order:** all three lists use recent activation, the Navigator's order,
  which stays stable while the overlay is open. `Space t t` lists running
  terminals before exited ones, by recent activation within each group.
- **Enter:** all three use the Navigator's rule and the verb `visit`: focus the
  destination where it is already visible, preferring the active pane, then
  the most recently activated pane; otherwise show it in the active pane.
- **Filter:** all three use the Navigator's fuzzy matching over name and path
  fields.
- **Preview:** on by default in all three. `Ctrl-t` still toggles it.
- **Tab:** unchanged; each row keeps the action menu of its kind.

### Key legend

A dimmed legend of the available keys is drawn inside the overlay, below the
list, following the design in `session_manager_key_discoverability.md`:

- It is pinned to the bottom of the overlay, so it does not move as the filter
  narrows the list.
- Rows take priority: when the overlay is too short for the rows and the
  legend, the legend compacts or disappears before any row is hidden.
- It lists only keys that act in the current state, and covers at least the
  movement, page, first/last, `Ctrl-t` preview, and `Delete` clear-filter keys.
- Key spellings come from the keymap registry and reflect remapping. This
  requires moving the generic list chords into the registry as a scope of
  their own, shared with the session manager's legend.

The title keeps only `Enter`, `Tab`, and `Esc`; `Ctrl-t toggle preview` moves
to the legend.

### Terminal IDs

Terminal IDs leave the list rows. `:terminal-send [id|name]` accepts an ID, so
the ID moves into the terminal pane title, for example:

```text
[terminal #3] Name or path [insert]
```

The ID remains a filter field in the Navigator and `Space t t`, so typing `3`
or `#3` still finds the terminal.

## Constraints

- The Session Manager (`Space Space`) keeps its own visual design; it shares
  only the legend convention and the registry scope for list chords.
- Dispatch, help, key hints and the legend read from the same keymap registry.
- The missing `[+]` in the buffer picker is tracked separately in
  `buffer_picker_modified_marker.md`.
- `context/reference/ui-vocabulary.md` (Session strip and Navigator) and the
  buffer, terminal and Navigator parts of `docs/user-guide.md` describe the
  shared layout, the legend, and the terminal ID in the pane title.

## Reproduction

1. Open two files, modify one, and start a terminal with `Space t n`.
2. Press `Space n`, `Esc`, `Space b b`, `Esc`, `Space t t`, `Esc`. Each overlay
   lays out, marks, and orders the same destinations differently, and none
   lists the `Ctrl-` chords it accepts.
