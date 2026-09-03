# Key hints stay in one column on a wide terminal

The key-hint popup lists one binding per line whatever the terminal width, so
a namespace with many continuations scrolls even when there is room on screen
for two columns of rows.

## Observed behavior

`draw_key_hints` in `src/ui.rs` already lays rows out in columns:

```rust
let columns = (inner_width / widest).max(1).min(rows.len());
```

`widest` is the key column plus a space plus the **longest** description among
the visible rows, clamped to `MIN_COLUMN_WIDTH` 36 and `MAX_COLUMN_WIDTH` 72,
plus a two-cell gap. Every `Space` and `Ctrl-w` menu contains at least one
description long enough to reach that clamp, so `widest` is 74 in practice and
a second column requires an inner width of 148 — a terminal 150 columns wide.
Below that the popup is one column, which is what is seen at ordinary widths.

The shared snapshot renderer used by an attached client
(`draw_snapshot_overlay`, `OverlayKind::KeyHints`) draws one row per line
unconditionally, so it is single-column at any width.

## Expected behavior

The popup uses two columns whenever the terminal is wide enough to render both
legibly, and falls back to one column when it is not. `Space` and `Ctrl-w`
menus are the cases that matter most, because they are the longest. Reaching
that width is partly a rendering decision and partly a writing one: the column
is as wide as the longest description in it, so a maximum description length
has to be chosen and the descriptions that exceed it rewritten.

Both renderers behave the same way. A reader attached to a persistent session
must not see a different layout from a standalone editor at the same width.

## The width budget, and what it costs

One hint column is the key column, a space, the description, and a two-cell
gap. The key column is `KEY_COLUMN_WIDTH` 12 unless a row's key text is wider,
in which case it grows to at most 20. Two columns therefore need

```
terminal width >= 2 * (key column + 1 + description + 2) + 2 borders
```

which, with a 12-cell key column, gives:

| Longest description | Terminal width needed for two columns |
| --- | --- |
| 72 (today's clamp) | 152 |
| 56 | 120 |
| 44 | 96 |
| 34 | 76 |

The decision is a maximum description length, and descriptions longer than it
are rewritten to fit rather than being allowed to force one column. Measured
over the 289 descriptions in `src/command.rs` that reach a hint row — 231
editor-command descriptions, mean 31 characters, and 58 colon-command
descriptions, mean 47 — the cost of each candidate is:

| Maximum | Descriptions that must be rewritten |
| --- | --- |
| 50 | 41 |
| 44 | 62 |
| 40 | 78 |
| 34 | 129 |

The suffixes `key_hint_description` appends — ` ›` for a namespace,
`  (exact)`, and `  unavailable: <reason>` or `  planned: <reason>` — cannot be
shortened by rewriting a description, so they are outside the budget and are
clipped when a row runs out of room.

## Points the fix has to settle

- **The maximum, and therefore the terminal width two columns start at.**
- **Whether the maximum is enforced.** A test that fails when a description
  exceeds it keeps the layout from silently regressing to one column the next
  time a command is added; without one the rewrite is a single-commit
  improvement that decays.
- **More than two columns.** The existing arithmetic already allows three or
  more on a very wide terminal. Keeping that is preferable to hard-coding two;
  the request is that two become reachable, not that more become impossible.
- **Row order.** Rows currently fill column-major — down the first column,
  then down the second. That should not change.

## Constraints

- Scrolling capacity already multiplies rows by columns
  (`KeyHintState::note_scroll_limit`), and the popup's height and the
  `1-N/total` range in its title are derived from the same numbers. They have
  to stay consistent with the column count actually drawn.
- The snapshot path publishes at most `ROW_LIMIT` (512) rows with an
  `omitted_rows` count, which is ample for a multi-column hint popup; the
  column layout is a rendering decision and stays in the frontend rather than
  becoming part of `OverlaySnapshot`.
- `context/reference/ui-vocabulary.md` describes overlays as
  presentation-neutral snapshots that frontends lay out; the register should
  say that the hint popup's column count is one of those frontend decisions.

## Regression coverage

Render the popup at a narrow and a wide width and assert the column count that
results, in both the standalone and the snapshot renderer. `tests/key_hints.rs`
and the rendering tests in `src/ui.rs` are the existing homes.
