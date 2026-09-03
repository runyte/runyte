---
title: "The green Select-mode cursor was too close to the blue Normal-mode cursor"
status: resolved
reported: 2026-08-11
resolved: 2026-08-11
legacy_commit: 02146e5
---

## Resolution

Commit 02146e5 (`Use orange cursors in Select mode`) changed the built-in
`ThemeDefinition::cursor_select` values in `Config::default`. They had all used
palette-local greens, which did not separate Select mode strongly enough from
the blue Normal cursor when many cursors were visible. Each built-in now uses
an orange already present in or suited to its palette: `base16` uses `#dc9656`,
`dark` uses `#f0a868`, `gruvbox` uses `#fe8019`, `light` uses `#953800`, and
`paper` uses `#d75f00`.

The renderer remains driven by the resolved `cursor_select` theme field; no
colour was hardcoded in `src/ui.rs`. The README custom-theme example and the
earlier cursor-colour resolution were updated to describe the new built-in
palette.

A later presentation revision added `selection_primary` as the light-orange
background paired with that cursor. It also limits pristine search results to
one orange primary cursor, while ordinary transformed multi-selections retain
an orange endpoint on every range.

`default-dark` and `default-light` later left that orange behind. Both now use
pink for Select — `#f07ab4` on the dark ground and `#a4276f` on the light one —
with a pink `selection_primary` beneath it (`#5e2e4d` and `#f2b8da`) and a more
saturated blue secondary selection (`#0b3f8c` and `#8fc6fb`). The reason the
orange was chosen still holds for every other bundled theme, which keeps it:
those themes use blue in Normal mode, so Select had to move away from blue. The
branded pair uses green in Normal and blue in Command, which leaves warm colours
free, and pink separates Select from both the brand-red Insert cursor and the
purple Replace cursor its green Normal forces.
`src/config.rs::tests::default_themes_use_a_pink_primary_selection_and_a_vivid_blue_secondary`
covers the pair; the orange list in
`built_in_search_selection_palettes_are_legible_and_role_distinct` covers the
rest.

`src/config.rs::tests::built_in_themes_use_mode_specific_cursor_colors` covers
the exact orange value for the themes that kept it.
`src/ui.rs::tests::editor_caret_uses_the_theme_color_for_each_mode` covers use
of the resolved Select cursor colour by the TUI.

Known limitation: an existing custom theme that explicitly configures a green
`cursor_select` remains green. Custom palette choices are intentionally not
rewritten.

## Report

The built-in themes used a green cursor in Select mode. Green was too close to
the blue Normal-mode cursor, particularly when distinguishing the primary
cursor in a multi-selection. Select-mode cursors were expected to be orange
instead.
