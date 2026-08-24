---
title: "Nordfox and Terafox are not available as built-in themes"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: dd02551
---

## Resolution

Commit `dd02551` (`Add Nordfox and Terafox themes`) fixed the theme inventory in
`Config::default`, which had no definitions for either requested Nightfox
palette and therefore gave the theme command and settings registry nothing to
discover or resolve under those names.

The change adds `nordfox_theme` and `terafox_theme` as additive built-in theme
definitions. Each maps the authoritative Nightfox background, foreground,
selection, syntax, diagnostic, and Git colors onto Runyte's presentation
roles. Nightfox's own blend ratios are used for the line-diff backgrounds;
Runyte-specific mode cursors and jump labels use red, orange, blue, and cyan
shades from the same palette. This keeps the source palettes recognizable
without importing Neovim highlight-group concepts into Runyte's smaller syntax
scope model. The existing `theme_names` path then makes both names available
to `:theme` and the registry-backed configuration picker. The README documents
the new built-ins and their Nightfox source.

Tests covering the behavior are
`config::tests::built_in_themes_are_listed_in_a_stable_order` and
`config::tests::fox_themes_follow_the_authoritative_nightfox_palettes` in
`src/config.rs`, plus
`settings::tests::registry_has_stable_unique_keys_ids_and_typed_configured_values`
in `src/settings.rs`. Run them with `cargo test`.

Known limitation: Runyte has fewer syntax roles than Nightfox has Neovim
highlight groups, so related upstream groups intentionally share the closest
Runyte scope color rather than reproducing every Nightfox highlight separately.

## Report

Runyte did not include built-in `nordfox` and `terafox` themes from Nightfox:
<https://github.com/EdenEast/nightfox.nvim>. Both themes were requested for
Runyte, together with test coverage for the addition.
