---
title: "Session manager keys are not discoverable"
status: resolved
reported: 2026-09-26
resolved: 2026-09-26
commit: 525facb
---

## Resolution

Commit `525facb` (`Name the session manager's keys in a legend and its Tab
menu`) resolves the issue. The session manager's `Ctrl-o`, `Ctrl-e` and
`Ctrl-g` were matched directly in `handle_list_key` in
`src/app/language_workflows.rs`, keyed on the list title, and every other list
key was a hard-coded `match` on key codes in the same function. None of them
was in the keymap registry, so nothing but the user guide could name them.

Lists now have keys in the registry. `Mode::List` in `src/command.rs` holds the
keys a filterable list reads while it owns input; the editor never enters that
mode. `list_bindings` in `src/keymap.rs` registers the generic list keys in its
global scope as new commands (`list-next`, `list-previous`, `list-page-down`,
`list-page-up`, `list-first`, `list-last`, `list-toggle-preview`,
`list-clear-filter`, `list-close`) and the manager's chords in a new
`BindingScope::SessionManager`: `Ctrl-o` to the existing
`open-session-directory`, `Ctrl-e` to a new `open-session-destinations`, and
`Ctrl-g` to `:git-worktrees`. A mode rather than a scope carries the list keys
because a scope inherits the global bindings of its mode: the manager scope
therefore inherits the list keys and none of the editor's. The new commands
are not in the `:` palette. `Keymap::list_binding` looks one key up, since a
list has no prefixes or counts for an input grammar to interpret, and
`handle_list_key` dispatches every bound key through it, including in reports.
Enter, Tab, Backspace, printable filter text and the empty-filter digits stay
the list's fixed grammar. Configured `keys.rebind` rules reach only the `Space`
and `Ctrl-w` namespaces, so these keys are not remappable, and spelling them
from the registry is what keeps legend and dispatch together.

`App::list_legend` in `src/app/presentation.rs` builds the legend from the
same bindings, naming each command's primary spelling (arrows, `Shift-Tab`,
paging keys and `Ctrl-c` are compatibility spellings) and pairing
`Ctrl-n/Ctrl-p move`, `Ctrl-d/Ctrl-u page` and `Home/End first/last`. An entry
appears only while its key acts: `1-9 attach` while the filter is empty in a
persistent editor, `Ctrl-e` while a running row is selected, `Delete clear
filter` while there is a filter, and a chord only while its command's platform
and capability allow it. `OverlaySnapshot` gained a `legend` field, and the
private protocol moved to version 60 for it and for the wire `Mode`.
`legend_lines`, `legend_height` and `draw_legend` in `src/ui.rs` serve both the
local list renderer and the snapshot renderer attached clients use: the legend
is pinned to the bottom, gives up its blank separator line and then itself
before any row is hidden, and never scrolls the list. `ListPicker::with_key_legend`
opts a list in; the manager is the first.

The title is `Sessions · Enter open · Tab actions · Esc close` (Enter was
`attach` on Unix and `visit` on Windows). The Tab menu keeps the row actions
and appends `SessionAction::OpenDirectory`, `Destinations` (running rows only)
and `Worktrees` under a non-selectable **Manager** heading, each with its chord
in the pinned trailing column. Choosing one runs the same target as the chord.
Beyond the report, a chord or entry whose command is unavailable (Git
worktrees outside a repository, the directory chooser in a standalone editor)
now refuses with the reason and leaves the manager open; `Ctrl-g` used to close
the manager before failing. The menu draws such an entry unavailable with the
reason, while the legend omits it.

Tests: `session_manager_legend_names_only_the_keys_that_act_now`,
`standalone_session_manager_legend_omits_persistent_only_keys` and
`session_actions_end_with_the_manager_chords_whichever_row_is_selected` in
`src/app/tests/workspace.rs`;
`list_keys_leave_typing_to_the_filter_and_editor_keys_to_the_editor` in
`src/keymap.rs`; `a_list_legend_gives_way_before_any_row` and
`session_manager_pins_its_key_legend_below_the_rows_in_both_renderers` in
`src/ui.rs`. The Windows expectations in
`src/app/tests/windows_session_manager.rs` were updated to the new title,
verb, legend and menus.

Known limitation: the Windows-only code and tests were not compiled for this
change; the development machine has no MinGW toolchain for the tree-sitter
grammars, so they rely on CI.

## Report

The session manager (`Space Space`, `:session-list`) answered four control
chords that nothing in the overlay mentioned:

- `Ctrl-o` opens the directory chooser (**Open directory…**), which visits or
  starts the persistent session of another directory.
- `Ctrl-e` opens the selected running session's inventory of open
  destinations.
- `Ctrl-g` opens the Git worktree workflow.
- `Ctrl-t` toggles the preview.

The title reads `Sessions · 1-9 attach · Tab actions` in a persistent editor
and `Sessions · Tab actions` in a standalone one. The Tab menu lists only
actions of the selected row (Open, Rename, Renumber, Close, Force close for a
running row; Open, Rename, Forget for a stopped row), so none of the four
chords can be found from inside the overlay. They are documented only in
`docs/user-guide.md`.

`Ctrl-o` also reads as a conflict: in buffers it is the jump-list step
backward. Inside the manager the overlay owns input first, so the jump list is
never reached and no jump is lost, but a user who knows only the buffer
meaning has no reason to try it here.

The chords are matched directly in `src/app/language_workflows.rs`, keyed on
the list title starting with `Sessions`, rather than through the keymap
registry.

### Expected behavior

The chords stay bound. `Ctrl-o` is kept: it is the conventional "open"
mnemonic, it belongs to the same family as `Ctrl-e` and `Ctrl-g`, and printable
keys are filter input, so any direct manager shortcut has to be a chord.

#### Title

The title keeps only the keys needed to use the overlay at all:

```text
Sessions · Enter open · Tab actions · Esc close
```

`1-9 attach` moves to the legend.

#### Key legend

A dimmed legend of the available keys is drawn inside the overlay, below the
session list:

- It is pinned to the bottom of the overlay rather than following the last
  row, so it does not move as the filter narrows the list.
- Session rows take priority. When the overlay is too short for the rows and
  the legend, the legend compacts or disappears before any row is hidden.
- It lists only keys that act in the current state. `1-9 attach` is shown only
  while the filter is empty and the editor is persistent. `Ctrl-e` is shown
  only while a running row is selected.
- It covers at least `Ctrl-o`, `Ctrl-e`, `Ctrl-g`, `Ctrl-t`, and `1-9`.
- Key spellings come from the keymap registry and reflect remapping, as help
  and key hints do. This likely requires moving the manager chords into the
  registry as a scope of their own.

#### Tab menu

The Tab menu gains manager-wide entries for the same actions, shown with their
keys:

```text
Open directory…      Ctrl-o
Open destinations    Ctrl-e
Git worktrees        Ctrl-g
```

These entries do not depend on the selected row. They appear below the row
actions, visually separated from them, whichever row is selected; the Explorer
Tab entry **Open persistent session here** is the existing precedent for a
view-wide action in a Tab menu. The destinations entry appears only when a
running row is selected.

The directory entry is named **Open directory…** to match the chooser it
opens. **Open path** was considered and rejected: the chooser opens only
directories, and the name sits next to the row action **Open**.

### Constraints

- Dispatch, help, key hints, the legend, and the Tab menu read from the same
  keymap registry.
- `context/reference/ui-vocabulary.md` and the session-manager part of
  `docs/user-guide.md` describe the legend and the new Tab entries.
- The Navigator, the buffer picker, and the terminal list adopt the same
  legend in `navigator_buffer_terminal_list_unification.md`, which moves the
  generic list chords into a registry scope shared with this legend. The
  Finder and the Open with picker have similar unlisted keys and are out of
  scope here; the legend is recorded as a general overlay convention in
  `context/reference/ui-vocabulary.md` once both issues land.

### Reproduction

1. Start Runyte in persistent mode: `runyte -a`.
2. Press `Space Space`. The title shows `1-9 attach` and `Tab actions` only.
3. Press `Tab`. The menu lists row actions only.
4. Close the menu and press `Ctrl-o`. The directory chooser opens, although
   nothing in the overlay mentioned it.
