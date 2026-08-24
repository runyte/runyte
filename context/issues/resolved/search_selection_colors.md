---
title: "Search selections showed too many cursors and did not distinguish the primary range"
status: resolved
reported: 2026-08-11
resolved: 2026-08-11
legacy_commit: d806c54
---

## Resolution

Commit d806c54 (`Clarify search selection presentation`) changed snapshot role
classification, which had treated every multi-selection head as a visible
caret and had no separate theme role for the primary selection body. Exact
search selections are now tagged with their buffer revision: while that tag is
current, the primary match uses the light-orange `selection_primary` body and
orange Select cursor, and secondary matches retain the theme's secondary
selection colour without cursor blocks. A selection motion changes the
revision, clears that search presentation, and restores the ordinary endpoint
cursors because the ranges have become editing selections rather than pristine
search results.

The theme schema gained `selection_primary`, with palette-local light-orange
values in the built-in themes and a compatibility fallback to `selection` for
custom themes. The input grammar also exposes a pending character command to
snapshot construction. That lets pending `r` use a red `ReplaceCaret` on every
selection head; `i`, `a`, and `c` continue into Insert mode, whose cursors are
all red.

A later palette audit kept the light and paper pairs and changed the neutral
secondary selection backgrounds in base16, dark, and gruvbox to palette-local
cool blues. All five built-in themes now use the same visual grammar: cool blue
for secondary matches, warm orange for the primary range and Select cursor,
and red for editing cursors. The audit also codified minimum text and cursor
contrast so future palette changes cannot silently make one of those roles
illegible.

A subsequent visual check on real terminal output found that the first
base16 and dark blues were still too close to their editor backgrounds. Those
two secondary backgrounds were raised in brightness while retaining better
than 4.5:1 foreground-text contrast; gruvbox, light, and paper were deliberately
left unchanged. The contrast test now also guards the background separation
that exposed the problem.

`src/snapshot.rs::tests::pristine_search_hides_secondary_carets_until_the_selection_moves`
covers pristine search roles and restoring ordinary cursors after a motion.
`src/snapshot.rs::tests::pending_replace_marks_every_selection_head_as_a_replace_caret`
covers red pending-replacement roles and applying the replacement.
`src/snapshot.rs::tests::a_multiselection_marks_its_complete_primary_range_separately`
covers primary and secondary selection-body roles.
`src/ui.rs::tests::editor_caret_uses_the_theme_color_for_each_mode` covers the
Select and replacement colour mapping.
`src/config.rs::tests::built_in_themes_add_palette_local_primary_selection_colors`
and
`src/config.rs::tests::built_in_search_selection_palettes_are_legible_and_role_distinct`
cover the exact built-in colours, role identity, and contrast.
`src/config.rs::tests::custom_theme_uses_fallbacks_and_accepts_overrides` covers
custom-theme fallback behavior.

Known limitation: a custom theme that omits `selection_primary` keeps its
existing selection colour for the primary body; it must set the new field to
opt into a distinct light-orange body.

## Report

Fresh search results should not draw a cursor on every selected match. The
primary match should have a light-orange selection with an orange cursor, while
all other matches should remain blue selections without cursors.

Ordinary Select mode should use the same light-orange selection and orange
cursor for the primary selection. Once a motion transforms search results, the
selection endpoints matter again and ordinary multi-selection cursors should
return.

Pressing `i`, `a`, or `c` on a multi-selection should show every resulting
Insert-mode cursor in red. Pressing `r` should show every selection cursor in
red while the editor waits for the replacement character, then return to Normal
mode after applying it.
