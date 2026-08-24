---
title: "The four Catppuccin flavours are not available as built-in themes"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: 23acaa2
---

## Resolution

Fixed by 23acaa2 "Add Catppuccin theme variants".

`Config::default` did not define any of Catppuccin's four flavours, so they
could not be resolved by name, selected from the theme setting, or persisted
as a usable built-in choice. A shared `catppuccin_theme` mapping now turns the
official palette roles into Runyte's editor, syntax, cursor, explorer, Git,
jump-label, selection, and side-by-side diff roles. `latte`, `frappe`,
`macchiato`, and `mocha` are inserted as independent built-ins and therefore
also pass through the existing custom-configuration merge and sorted theme
discovery paths.

The core palette values and syntax accents follow Catppuccin. Runyte's
whole-cell selection and diff backgrounds use palette-local tints instead of
the strong text accents, because those fields fill text cells and must preserve
foreground contrast. Latte's jump labels use its mauve and red rather than its
yellow and peach because the latter pair does not meet Runyte's light-background
legibility boundary. The mode cursors retain Runyte's established blue, red,
and orange distinction; Latte's Select cursor uses a darker peach-derived
orange for the same contrast reason.

Tests: `all_catppuccin_flavours_resolve_with_their_official_core_palette`,
`built_in_themes_use_mode_specific_cursor_colors`,
`built_in_themes_add_palette_local_primary_selection_colors`,
`built_in_search_selection_palettes_are_legible_and_role_distinct`,
`built_in_themes_use_palette_specific_blue_directory_colors`, and
`built_in_jump_labels_are_legible_and_close_in_weight` in `src/config.rs`; and
`registry_has_stable_unique_keys_ids_and_typed_configured_values` in
`src/settings.rs`.

Known limitation: these are fixed built-in palettes; Runyte does not import or
track later upstream Catppuccin palette changes automatically.

## Report

All four Catppuccin themes from https://github.com/catppuccin/nvim were
requested as built-in themes:

- latte
- frappe
- macchiato
- mocha
