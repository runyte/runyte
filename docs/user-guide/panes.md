## Panes

### Pane keys

Pane commands live under `Space w`, with `Ctrl-w` as the compatibility
spelling. From Insert and Replace modes, `Ctrl-w` reaches focus movement
(`h/j/k/l`), pane moves (`H/J/K/L`), `w`, `x`, `n`, `p`, and `a`. Terminal Insert adds `v/s` and
`f/z` (see [Terminals](terminals.md#terminals)).

| Key | Action |
| --- | --- |
| `Space w v` / `Space w s`, or `Ctrl-w v/s` | Vertical/horizontal splits; from a terminal, the new pane opens the working-directory explorer |
| `Space w…` or `Ctrl-w…` | Window focus, close, next, and only-window operations |
| `Space w x` or `Ctrl-w x` | Exchange the active pane's complete content with the previously focused pane, following the content to its new position |
| `Space w H/J/K/L` or `Ctrl-w H/J/K/L` | Move the active pane to the left/bottom/top/right edge, spanning the full height or width |
| `Space w =` | Equalize pane widths, then pane heights within each column |
| `Space w f` / `Space w z`, or `Ctrl-w f` / `Ctrl-w z` | Toggle the full-screen pane / the centred Zen viewport |
| `Ctrl-h/j/k/l` | Move between panes without a prefix, when `editor.fast_pane_keys` is on |
| `:vsplit [path]` / `:hsplit [path]` (`:split`) | Create a side-by-side / stacked split |
| `:resize` | Preview pane resizing and extension; Enter applies, Escape cancels |

Borders can also be dragged with the mouse. The key-hint popup after
`Space w` lists every window command.

Hold Alt and drag with the left mouse button inside a pane to rearrange it. A
thick **pane drop outline** shows where it will land when you release. Where
you drop it matters:

- The outer two columns or one row of the layout move it to that whole edge.
- The outer quarter of another pane splits that pane to make room.
- The middle of another pane swaps their contents.

Shared borders resize even with Alt held. Alt-left-press inside a terminal pane
belongs to Runyte and is not forwarded to the child, even if the child asks for
mouse reporting. Pane moves keep the pane's content and terminal session. They
are not available while Zen or full-screen maximizes a pane.

Hold Shift and Alt when you start a left-drag to **extend** the pane in place.
Drag outside one of its sides, and the outline snaps that side to the far edge
of the pane under the pointer. Release to apply it, or press Escape to cancel.

The other sides stay where they are. Neighbours shrink if they can stay one
rectangle. Covered panes, and panes that would be cut into separate pieces,
close as with `:wc`. Adjacent panes grow into the free space. Buffers, unsaved
edits and terminal sessions stay available. If the result can't form a
rectangular layout, no outline appears and release changes nothing. Shared
borders still resize normally.

For example, starting with `AA / BC`:

- extending C upward gives `AC / BC`
- extending B right gives `AC / BB`
- extending B upward leaves B filling the editor

With `AAA / BCD`, extending C upward closes A and grows B and D. You end up
with three full-height columns, `BCD`.

**Examples:**

| Keys | Result |
| --- | --- |
| `Space w v` then `Space f` | Open a file beside the current one |
| `:resize`, ten `l` presses, Enter | Move the initially selected vertical edge ten columns right |
| `Space w =` | Level every split again |
| `Ctrl-w l` in Insert mode | Move right without leaving Insert mode |

### Keyboard resize preview

`:resize` briefly fades pane contents away and replaces them with a labelled layout preview. The active pane
has a tinted background and its selected edge has a bright handle. The status
line lists the controls; the interaction line keeps Apply and Cancel visible.

- `Tab` / `Shift-Tab` cycles clockwise / counterclockwise through movable edges,
  initially choosing the right edge when available.
- `h/l` or Left/Right moves a vertical edge one column; `k/j` or Up/Down moves
  a horizontal edge one row. Directions move the edge literally: moving a left
  edge right shrinks the active pane. Perpendicular keys leave it unchanged.
- `H/J/K/L` or Shift+arrows extends the active pane in that direction to the
  nearest valid neighbouring far edge, using the same rules as Shift-Alt-drag.
  Every affected pane is previewed. Covered or fragmented pane views disappear;
  their buffers, unsaved edits and terminal sessions survive acceptance.
- `u` undoes a preview change, including an extension (up to 256 changes).
- `Enter` applies the complete draft. `Esc` discards it.

The directory tree's right edge, also reachable from an adjacent pane's left
edge, can be resized but cannot be absorbed by an extension. Fullscreen and Zen
must be left before resizing. A changed screen size, underlying layout or
frontend attachment cancels the draft. Terminal processes retain their actual
sizes throughout the preview and receive the new sizes only after acceptance.
Mouse input and pasted text do not edit anything while this mode owns input.

The former `:resize-left`, `:resize-right`, `:resize-top` and `:resize-bottom`
commands have been replaced by `:resize`.

### Splitting

- **A file** splits into two panes showing the same buffer.
- **An explorer** splits the same way: same listing, same row. No key opens the
  entry under the caret in a split. Navigate the new pane elsewhere and it
  gets an explorer of its own.
- **A terminal** is the exception. The terminal stays where it is, live or in
  review. The new pane opens the working directory as an explorer, as
  `Space E` would.

### Fast pane keys

Set `editor.fast_pane_keys: true` to move between panes with the bare
`Ctrl-h/j/k/l` (left, down, up, right), as a tmux user would. It works in
Normal, Select, Insert, and a live terminal.

It is off by default because those keys are not free:

| Key | Normally |
| --- | --- |
| `Ctrl-j` | Insert newline (Insert mode) |
| `Ctrl-k` | Delete to end of line (Insert mode) |
| `Ctrl-h` | Backspace, in most terminal programs |
| `Ctrl-l` | Clear the screen, in most terminal programs |

With the setting on, a program in a terminal never receives those four keys.
Both prefixed spellings keep working. Help and the key-hint popup show the keys
currently in use. Configured bindings still decide the action.

On native Windows, Runyte asks for keyboard reporting that keeps Ctrl chords
separate from Backspace and Enter. Fast pane keys also work in the explorer,
where the physical Backspace and Enter keys still navigate and open.

### Equalizing

`Space w =` evens out splits after `:resize` or dragging a border.

1. Every pane gets the same width.
2. Every pane sharing a column gets the same height.

Only the boundaries move. The panes stay in the same arrangement. A pane
spanning the full width above a row of others keeps spanning it, and the row
below is levelled.

### Zen and fullscreen

| | `:zen` (`Space w z`) | `:fullscreen` (`Space w f`) |
| --- | --- | --- |
| Maximizes the active pane | Yes | Yes |
| Text width | Centred, up to `editor.zen_width` (100) cells; a narrower terminal uses all cells | Unchanged: laid out exactly as in a split |
| With only one pane | Works, because it changes the text width | Leaves the view unchanged and reports `only one pane` |
| Pane title tag | `[zen]` | `[fullscreen]` |

- The buffer, selections, undo history, and split tree are unchanged. Toggling
  again restores the exact previous layout.
- Terminal panes use the same centred width and retain the full available
  height. The running program receives the new terminal size, including in
  alternate-screen apps. Leaving Zen restores the pane dimensions. Existing
  terminal output is not reflowed when resized.
- Soft wrap keeps the same settings, including the rendered Markdown override.
- With several panes, the two views share one state. Asking for the other view
  switches to it. Only the view currently showing toggles off.
- While a pane is maximized, splitting, closing, directional focus and pane
  cycling are unavailable. Keys only reach the maximized pane, so you always
  type into what you see.
- The title tag follows the `[+]` and `[RO]` markers. An ordinary pane has
  neither tag.
- Both cover the directory tree temporarily, and zen hides the session strip.

<!-- navigation -->
[Contents](README.md) · [Previous: Files and buffers](files-and-buffers.md) · [Next: Terminals](terminals.md)
