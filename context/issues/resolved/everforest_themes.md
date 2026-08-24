---
title: "Everforest dark and light contrast variants were unavailable"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: 2d61c38
---

## Resolution

Commit 2d61c38 (`Add Everforest theme variants`) fixed this. `Config::default`
only registered Runyte's original five built-in themes, so no Everforest name
could be resolved, listed in the theme settings, or persisted as a valid
choice. The built-in registry now adds six definitions through a shared
Everforest foreground constructor and per-contrast background palettes. This
keeps the dark and light syntax semantics consistent while preserving the
upstream hard, medium, and soft background differences.

Runyte deliberately uses Everforest's blue and yellow background roles for
secondary and primary selections instead of assigning the upstream
`bg_visual` colour to both. That preserves Runyte's established distinction
between cool secondary selections and warm primary selections. The light jump
labels similarly use darker shades of Everforest purple because the upstream
accent is not legible enough as small text on the light backgrounds.

Tests cover the behavior in
`src/config.rs::tests::everforest_variants_use_the_upstream_palettes_and_runyte_roles`,
`src/config.rs::tests::built_in_themes_are_listed_in_a_stable_order`, and
`src/settings.rs::tests::registry_has_stable_unique_keys_ids_and_typed_configured_values`.

## Report

All six variants of the [Everforest theme](https://github.com/sainnhe/everforest)
were requested:

- dark and light palettes;
- hard, medium, and soft contrast levels.
