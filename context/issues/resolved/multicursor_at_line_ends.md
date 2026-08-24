---
title: "Multi-cursor cannot extend through short lines at a line end"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: a33b2e6
---

## Resolution

Commit `a33b2e6` ("Add padded multicursor command") adds the Runyte-grammar
`V` command while leaving `C` and `Alt-C` unchanged. `V` adds a cursor to the
immediately following row and uses one transaction to pad short or empty rows
with spaces until the requested visual column exists. It accounts for tabs and
wide Unicode characters, so the added carets align with the rendered display
column rather than only a character index. Vim grammar keeps its existing
uppercase-`V` visual-line behavior.

The binding is documented in `README.md` and
`context/reference/helix-keymap-v1.md`. `padded_multicursor_adds_carets_on_short_and_empty_rows`
and `padded_multicursor_uses_display_columns_for_tabs_and_wide_characters` in
`tests/selection.rs` cover the requested padding behavior and display-width
edge cases.

## Report

At the end of a line followed by shorter lines, `C` extended a multicursor
only to lines of equal or greater length.

Example, where `X` marks a cursor:

```
This is a longer lineX
This is shorter

One line above was evXen empty
```

The requested alternative pads the shorter lines with spaces so every cursor
lands in the same column:

```
This is a longer lineX
This is shorter      X
                     X
One line above was evXen empty
```

`C` keeps its existing behavior; `V` adds the padded variant.
