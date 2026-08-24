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

`src/config.rs::tests::built_in_themes_use_mode_specific_cursor_colors` covers
the exact orange value for every built-in theme.
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
