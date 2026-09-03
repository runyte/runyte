---
title: "Key-hint columns differ between standalone and persistent modes"
status: resolved
reported: 2026-09-03
resolved: 2026-09-03
commit: b25e96b
---

## Resolution

Commit `b25e96b` (`Share responsive key hint layout`) fixed the mismatch. The
standalone `draw_key_hints` function had owned its column arithmetic while
`draw_snapshot_overlay` treated every transported hint as a separate screen
row. `WorkspaceHost::prepare_frame_with_hints` also removed the rows before
the current scroll offset, which left an attached frontend without the full
menu needed to measure a stable grid.

`key_hints::key_hint_layout` now resolves the key and complete-description
widths, one-to-three-column count, capacity, bounded offset, visible row count,
column-major height, and popup height for both frontend paths. The host keeps
the bounded full hint inventory in the presentation-neutral snapshot and uses
the existing scroll anchor to carry the requested first row; the attached TUI
then applies the shared calculation without serializing a column count.

`key_hints::key_hint_description` composes namespace, exact-binding, static
availability, and live capability markers before measuring terminal cells.
Explanatory descriptions that would exceed 44 cells use the semantic command
name already held by the command registry as compact hint-specific wording;
long live reasons use a short capability-specific reason. This deliberately
keeps full prose in help and command-palette surfaces and does not create a
second binding inventory. The exact-binding marker is `=` in the compact hint
surface rather than the longer `(exact)` spelling.

Regression coverage is in `src/key_hints.rs`:
`responsive_layout_uses_only_complete_columns`,
`responsive_layout_shares_capacity_offset_and_column_major_height`, and
`every_complete_hint_description_fits_forty_four_terminal_cells`. The last
test covers all 289 editor and colon command descriptions; registry and
synthetic rows additionally exercise namespace, exact, planned, unsupported,
and capability-unavailable forms. Rendering coverage is in `tests/key_hints.rs`:
`standalone_and_attached_hints_share_responsive_grid_boundaries` and
`attached_hint_scrolling_uses_the_shared_multicolumn_capacity`.

Known limitation: a terminal narrower than one complete capped row remains a
one-column emergency case and can clip at the terminal edge.

## Report

The key-hint popup uses the terminal width in standalone mode, but an attached
client renders the same hints in one column at every width. A wide standalone
terminal can therefore show two or three columns while a persistent session at
the same geometry scrolls through a single column.

## Observed behavior

`draw_key_hints` in `src/ui.rs` already lays standalone rows out in columns:

```rust
let columns = (inner_width / widest).max(1).min(rows.len());
```

The standalone renderer measures the widest key field and rendered
description. It clamps their combined width to `MIN_COLUMN_WIDTH` 36 and
`MAX_COLUMN_WIDTH` 72, adds a two-cell column gap, and divides the popup's
inner width by that stride. Every `Space` and `Ctrl-w` menu contains a row wide
enough to reach the upper clamp, so their effective stride is 74 cells. They
reach two columns at a 150-cell editor width and three at 224 cells. The same
arithmetic can produce still more columns on an unusually wide terminal.

The shared snapshot renderer used by an attached client
(`draw_snapshot_overlay`, `OverlayKind::KeyHints`) instead draws one overlay
row per screen row unconditionally. The persistent-session host publishes the
same semantic key-hint rows, but its attached TUI never applies the standalone
column layout to them.

The upper width clamp can also make a standalone column narrower than its
longest rendered row. Ratatui then clips the description at the cell boundary.
The grid appears to fit because the text, rather than the requested number of
columns, pays for the mismatch.

## Expected behavior

Standalone and attached frontends use one shared key-hint layout calculation.
At the same editor geometry and with the same rows, they choose the same column
count, popup height, visible rows, scroll range, and column-major row order.

The responsive layout has at most three columns:

- use three when three complete hint rows and their gaps fit;
- otherwise use two when two complete rows and their gap fit;
- otherwise use one.

The renderer never clips a description merely to gain another column. If all
of a candidate column's content does not fit, it chooses one fewer column. A
terminal too narrow for one complete capped row is the unavoidable emergency
case; it remains one-column and may clip at the terminal edge rather than
changing the ordinary width budget.

The complete visible description field has a maximum width of 44 terminal
cells. The limit applies after adding the namespace marker, the exact-binding
marker, and any planned, unsupported, or capability-unavailable suffix. Text
that exceeds it is rewritten at its source or given compact hint-specific
wording; it is not silently truncated by the renderer. Measure terminal cells,
not bytes or Unicode scalar values.

Enforce the 44-cell maximum with a test over every hint row and every suffix
variant that can be produced from registry and capability metadata. This is a
source invariant, not only a clamp in `draw_key_hints`, so a newly added
description cannot silently make the popup wider or become clipped.

## Width calculation

For a uniform grid, define:

- `K` as the widest rendered key field in the current rows, clamped from
  `KEY_COLUMN_WIDTH` 12 through the existing maximum 20;
- `D` as the maximum complete visible description width, 44;
- one cell between the key and description;
- `G` as the two-cell gap reserved per column by the current equal-width
  layout;
- two cells for the popup borders.

For `N` columns, the required terminal width is therefore:

```text
required_width(N) = 2 + N * (K + 1 + D + G)
                  = 2 + N * (K + 47)
```

The selected count is the largest `N` from three down to one whose required
width fits, also bounded by the number of rows. Using the common 12-cell key
field and the longest allowed 20-cell field gives:

| Columns | `K = 12` | `K = 20` |
| --- | ---: | ---: |
| 1 | 61 | 69 |
| 2 | 120 | 136 |
| 3 | 179 | 203 |

This makes a conventional 120-cell terminal reach two columns and a roughly
180-cell terminal reach three without trimming any capped description. The
actual threshold remains content-aware when the current menu needs a wider
key field or all of its rendered descriptions are shorter than the cap.

The current inventory contains 289 command descriptions that can reach a hint
row: 231 editor-command descriptions and 58 colon-command descriptions. The
previous source-only measurement found 62 longer than 44 characters. That is
only a lower bound for the rewrite because the enforced limit applies to the
complete rendered description, including markers and availability text; the
implementation must remeasure terminal-cell widths after composing those
forms.

## Constraints

- Scrolling capacity multiplies content rows by the selected column count.
  `KeyHintState::note_scroll_limit`, popup height, the visible slice, and the
  `1-N/total` title range must all use the same result.
- Rows fill column-major: down the first column, then down the second, then the
  third. Responsive layout must not change that order.
- The snapshot path publishes at most `ROW_LIMIT` (512) rows with an
  `omitted_rows` count, which is ample for this popup. Column count remains a
  frontend decision and does not become part of `OverlaySnapshot`.
- `context/reference/ui-vocabulary.md` describes overlays as
  presentation-neutral snapshots that frontends lay out. It must record that
  key-hint column count is one of those frontend decisions.
- Key dispatch, help, and hints continue to read descriptions from the shared
  command and keymap registries. Compact hint wording must not introduce a
  second binding inventory.

## Regression coverage

Render representative `Space` and `Ctrl-w` menus on both the standalone and
snapshot paths. Cover a width immediately below and at each two- and
three-column threshold, and assert equal column counts, column-major order,
visible capacity, title range, and absence of clipped descriptions. Exercise a
menu whose key field is 12 cells and one that makes it wider.

Add an inventory test for the 44-cell complete-description limit, including
namespace, exact, planned, unsupported, and capability-unavailable variants.
`tests/key_hints.rs` and the rendering tests in `src/ui.rs` are the existing
homes.
