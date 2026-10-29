## Panes

### Pane keys

Pane commands live under `Space w`, with `Ctrl-w` as the compatibility
spelling. From Insert and Replace modes, `Ctrl-w` reaches focus movement
(`h/j/k/l`), pane moves (`H/J/K/L`), `w`, `x`, `n`, `p`, and `a`; Terminal Insert adds `v/s` and
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
| `:resize-right +/- N` | Grow or shrink the active pane at its right edge by `N` terminal cells |
| `:resize-left +/- N` | Grow or shrink the active pane at its left edge by `N` terminal cells |
| `:resize-top +/- N` | Grow or shrink the active pane at its top edge by `N` terminal cells |
| `:resize-bottom +/- N` | Grow or shrink the active pane at its bottom edge by `N` terminal cells |

Borders can also be dragged with the mouse. The key-hint popup after
`Space w` lists every window command.

Hold Alt and drag with the left mouse button inside a pane to rearrange it.
A thick **pane drop outline** shows where it will land on release. The outer
two columns or one row of the layout move it to that whole edge; the outer
quarter of another pane splits that pane to make room; the middle of another
pane swaps their complete contents. Shared borders resize even with Alt held.
Alt-left-press inside a terminal pane belongs to Runyte and is not forwarded
to the child, even when the child requests mouse reporting. Pane moves preserve
the pane's content and terminal session, and are unavailable while Zen or
full-screen maximizes a pane.

Hold Shift and Alt when starting a left-drag to **extend** the pane in place.
Drag outside one of its sides: the outline snaps that side to the far edge
of the pane under the pointer. Release to apply it, or press Escape to cancel.
The other sides stay anchored. Neighbours shrink where they can remain one
rectangle; covered panes, or panes that would be split into separate pieces,
close as with `:wc`. Adjacent panes grow into any space left by those closures.
Buffers, unsaved edits, and terminal sessions remain available. If the result
cannot form a rectangular split layout, there is no outline and release leaves
the layout unchanged. Shared borders still resize normally.

For example, starting with `AA / BC`, extending C upward gives `AC / BC`;
extending B right gives `AC / BB`; extending B upward leaves B filling the
editor. With `AAA / BCD`, extending C upward closes A and grows B and D,
leaving three full-height columns `BCD`.

**Examples:**

| Keys | Result |
| --- | --- |
| `Space w v` then `Space f` | Open a file beside the current one |
| `:resize-right + 10` | Make the active pane 10 cells wider |
| `Space w =` | Level every split again |
| `Ctrl-w l` in Insert mode | Move right without leaving Insert mode |

### Splitting

- **A file** splits into two panes showing the same buffer.
- **An explorer** splits the same way: same listing, same row. No key opens the
  entry under the caret in a split. Navigate the new pane elsewhere and it
  gets an explorer of its own.
- **A terminal** is the exception. A terminal is pane content, not a buffer,
  so the pane still holds the buffer it showed before; that buffer is history,
  not something you asked to see twice. The terminal stays where it is, live
  or in review, and the new pane opens the working directory as an explorer,
  as `Space E` would.

### Fast pane keys

Set `editor.fast_pane_keys: true` to move between panes with the bare
`Ctrl-h/j/k/l` — left, down, up, right — as a tmux user would. It works in
Normal, Select, Insert, and a live terminal.

It is off by default because those keys are not free:

| Key | Normally |
| --- | --- |
| `Ctrl-j` | Insert newline (Insert mode) |
| `Ctrl-k` | Delete to end of line (Insert mode) |
| `Ctrl-h` | Backspace, in most terminal programs |
| `Ctrl-l` | Clear the screen, in most terminal programs |

With the setting on, a terminal's child never receives those four keys. Both
prefixed spellings keep working, and help and the key-hint popup describe
whichever set is in force. Configured bindings still decide the action.

On native Windows, Runyte requests keyboard reporting that keeps Ctrl chords
distinct from Backspace and Enter. Fast pane keys also work in the explorer,
where the physical Backspace and Enter keys keep directory navigation and
opening.

### Equalizing

`Space w =` levels splits after `:resize-*` or a dragged border has skewed
them.

1. Every pane gets the same width.
2. Every pane sharing a column gets the same height.

Only the boundaries move; which pane sits beside or above which stays the
same. A pane spanning the full width above a row of others keeps spanning
it, and the row below is levelled.

### Zen and fullscreen

| | `:zen` (`Space w z`) | `:fullscreen` (`Space w f`) |
| --- | --- | --- |
| Maximizes the active pane | Yes | Yes |
| Text width | Centred, up to `editor.zen_width` (100) cells; a narrower terminal uses all cells | Unchanged: laid out exactly as in a split |
| With only one pane | Works, because it changes the text width | Leaves the view unchanged and reports `only one pane` |
| Pane title tag | `[zen]` | `[fullscreen]` |

- The buffer, selections, undo history, and split tree are unchanged. Toggling
  again restores the exact previous layout.
- Soft wrap keeps the same settings, including the rendered Markdown override.
- With several panes, the two views share one state: asking for the other
  switches to it, and only the view actually showing toggles off.
- While a pane is maximized, splitting, closing, directional focus, and pane
  cycling wait until it is toggled off. The maximized pane is the only one keys
  can reach, so what is on screen and what receives typing cannot diverge.
- The title tag follows the `[+]` and `[RO]` markers. An ordinary pane has
  neither tag.
- Both cover the directory tree temporarily, and zen hides the session strip.

<!-- navigation -->
[Contents](README.md) · [Previous: Files and buffers](files-and-buffers.md) · [Next: Terminals](terminals.md)
