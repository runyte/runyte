# Navigator, buffer picker, and terminal list use three different designs

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
