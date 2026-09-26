# Session manager keys are not discoverable

The session manager (`Space Space`, `:session-list`) answers four control
chords that nothing in the overlay mentions:

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

## Expected behavior

The chords stay bound. `Ctrl-o` is kept: it is the conventional "open"
mnemonic, it belongs to the same family as `Ctrl-e` and `Ctrl-g`, and printable
keys are filter input, so any direct manager shortcut has to be a chord.

### Title

The title keeps only the keys needed to use the overlay at all:

```text
Sessions · Enter open · Tab actions · Esc close
```

`1-9 attach` moves to the legend.

### Key legend

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

### Tab menu

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

## Constraints

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

## Reproduction

1. Start Runyte in persistent mode: `runyte -a`.
2. Press `Space Space`. The title shows `1-9 attach` and `Tab actions` only.
3. Press `Tab`. The menu lists row actions only.
4. Close the menu and press `Ctrl-o`. The directory chooser opens, although
   nothing in the overlay mentioned it.
