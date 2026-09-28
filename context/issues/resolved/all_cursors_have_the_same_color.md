---
title: "All cursors have the same color in a multi-selection"
status: resolved
reported: 2026-09-28
resolved: 2026-09-28
commit: c5f05ae
---

## Resolution

Commit `c5f05ae` (`Distinguish primary and secondary carets`) separated the
caret roles in `SelectionRoles::role_at`. Previously, every head outside the
special Select-mode primary case received `TextRole::Caret`, and
`text_run_style` painted that role with the active mode color. In Select mode,
`PrimaryCaret` and `Caret` also resolved to the same color. Pending replacement
used one `ReplaceCaret` role for all heads. Consequently, caret blocks could
not identify the primary selection, particularly when selections were points.

`SelectionRoles::role_at` now assigns `SecondaryCaret` to nonprimary heads in
every mode and during pending replacement. It compares the head offset of the
primary range so two adjacent ranges with one shared head still paint that
cell as primary. The primary keeps its existing mode or pending-replacement
color. The theme's `cursor_secondary` role paints other caret blocks and
defaults to `foreground` for existing custom themes. At the frontend boundary,
the resolved terminal color is checked against primary caret colors; when a
limited terminal palette collapses two roles, an available color is chosen.
The bundled client protocol advances to version 64 for the new semantic role
and theme field.

Covered by `selection_role_walk_matches_range_semantics_at_boundaries_and_after_a_backward_jump`
and `pending_replace_distinguishes_the_primary_caret` in `src/snapshot.rs`;
`rotating_a_multiselection_moves_the_mode_colored_caret`,
`bundled_palette_secondary_carets_survive_terminal_color_depths`, and
`custom_theme_secondary_caret_avoids_equivalent_ansi_and_rgb_colors` in
`src/ui.rs`; and
`built_in_secondary_carets_are_visible_and_distinct_from_primary_carets` and
`custom_theme_cursor_colors_use_mode_specific_fallbacks` in `src/config.rs`.

## Report

In a multi-selection, every visible caret used the same color. The primary
caret could not be identified by its caret color, especially when the
selections were points with no range background. The primary caret was
expected to be visually distinct from secondary carets while the selection
ranges and editing behavior stayed unchanged.

Reproduction: create multiple carets in an editable buffer and compare their
colors. The reported example is shown in the [GitHub attachment](https://github.com/user-attachments/assets/5ac3160b-20c4-496d-b30e-eda011396b2d).
