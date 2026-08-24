---
title: "Dark themes did not distinguish Insert mode and no theme distinguished Select mode"
status: resolved
reported: 2026-08-11
resolved: 2026-08-11
legacy_commit: 620e884
---

## Resolution

Commit `620e884` (`add configurable select cursor colors`) extended
`ThemeDefinition` and resolved `Theme` with a separate `cursor_select` value.
`TuiTheme` maps Normal, Insert, and Select modes to `cursor_normal`,
`cursor_insert`, and `cursor_select` respectively instead of treating Normal
and Select as one mode. A later visibility change uses the same role for the
global status-line background, making the mode visible across the screen.

Every built-in theme supplies palette-local values. Insert is red. Select was
initially green, then changed to orange after the green proved too close to the
blue Normal cursor in multi-selections. This later palette adjustment leaves
the original `commit:` value above unchanged. The values are not hardcoded in
rendering. Custom themes may set each cursor color independently. Omitted
Normal, Insert, and Select values now use the theme's `accent`, `error`, and
`warning` roles respectively, so a minimal custom theme also begins with three
mode colours.

Covered by `config::tests::built_in_themes_use_mode_specific_cursor_colors`
and `config::tests::custom_theme_cursor_colors_fall_back_to_semantic_mode_colors` in
`src/config.rs`, and
`ui::tests::editor_caret_uses_the_theme_color_for_each_mode` and
`ui::tests::status_row_follows_the_caret_color_in_each_mode` in `src/ui.rs`.

## Report

The `paper` and `light` themes used a red cursor in Insert mode and a blue
cursor in Normal mode, while dark themes did not change cursor color for
Insert mode. Dark themes were expected to use a red Insert cursor as well.

Select mode also needed its own green cursor color in every theme. Each theme
was expected to carry an adjustable value rather than relying on one color
hardcoded across themes.
