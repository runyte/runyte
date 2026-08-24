---
title: "gw labels did not reflect proximity or the exact visible viewport"
status: resolved
reported: 2026-08-15
resolved: 2026-08-15
legacy_commit: dc62e07
---

## Resolution

Implemented in commit `dc62e07`, "Refine gw with proximity-ranked jump labels".

`JumpLabels::new` previously assigned fixed two-character labels in document
order, while `App::label_visible_words` treated hard-scrolled rows as extending
indefinitely to the right. The renderer then relied on terminal clipping, so a
word beyond the pane's right edge could receive an unreachable label. Ordinary
text runs also retained their syntax colours while labels were active, leaving
the hints without a distinct visual layer.

Jump targets are now ranked by weighted distance from the cursor in the same
projected screen-row space used for folds, soft wraps, and diff filler. The
prefix-free label generator keeps the closest targets on single red keys and
expands farther keys into two-character green labels. Typing a green prefix
removes unrelated hints and presents the surviving suffixes as red immediate
keys at their target cells. Snapshot state marks only the active pane for
dimming, and the terminal renderer maps its ordinary text to the theme's
`muted` colour while preserving backgrounds and label colours. Themes gained
an optional `jump_label_immediate` role, with `error` as the compatibility
fallback, and the private frame protocol moved to version 8 for the new state
and colour roles.

A later visual refinement replaced the built-in themes' mixed green hues with
one neon-cyan hue. The secondary character is darker than the primary on dark
backgrounds and lighter on light backgrounds, retaining contrast while making
the pair read as one colour.

Jump dimming later gained its own optional `jump_text_muted` theme role rather
than changing the general `muted` colour. It falls back to `muted`; the built-in
`light` and `paper` themes use `#a8adb2` and `#aaaaaa` respectively. The private
frame protocol moved to version 9 with the additional resolved colour.

Hard-scrolled target columns and available label widths are now measured in
terminal cells from the visible origin, including tabs and wide characters.
Labels that would cross the right edge are rejected and assignment is
regenerated, so a target is offered only when its actual one- or two-cell label
fits. Soft wraps continue to use their projected segment boundaries. Runyte
deliberately retains its existing interaction choices: a sole target still
shows a label, and unmatched input cancels the jump rather than being replayed
as an editor command.

Covered by
`src/jump_labels.rs::tests::the_twenty_seventh_target_expands_the_farthest_single_key`,
`src/jump_labels.rs::tests::narrowing_hides_other_labels_and_moves_red_suffixes_to_the_target`,
`src/jump_labels.rs::tests::labels_that_do_not_fit_are_removed_and_assignment_is_regenerated`,
`src/app.rs::tests::goto_word_gives_distant_targets_prefix_free_two_key_labels`,
`src/app.rs::tests::goto_word_keeps_a_fitting_one_key_target_at_the_right_edge`,
`src/app.rs::tests::goto_word_drops_a_two_key_target_that_crosses_the_right_edge`,
`src/app.rs::tests::goto_word_right_edge_is_measured_in_terminal_cells`,
`src/app.rs::tests::goto_word_right_edge_accounts_for_tabs`,
`src/app.rs::tests::goto_word_excludes_words_past_a_horizontally_scrolled_view`,
`src/snapshot.rs::tests::jump_dimming_belongs_only_to_the_active_pane`,
`src/ui.rs::tests::jump_labels_paint_over_the_words_they_name`,
`src/ui.rs::tests::distant_jump_labels_use_two_neon_cyans_then_narrow_to_one_red_key`,
and
`src/config.rs::tests::built_in_jump_labels_are_red_and_one_neon_cyan_hue`,
`src/config.rs::tests::light_and_paper_use_a_lighter_gray_only_while_jump_labels_are_active`,
and `src/config.rs::tests::an_older_theme_uses_muted_for_jump_dimming`.

Known limitation: words shorter than two characters, words whose first two
characters do not each occupy one terminal cell, and targets beyond the
two-character alphabet's 676-label capacity remain unlabelled. This preserves
the overlay's width and the established `gw` eligibility boundary.

## Report

The `g w` word-hopping display used two coloured letters at the beginning of
every eligible word while leaving all buffer colours unchanged. The preferred
behaviour was the visual and narrowing model used by NeoVim's
`smoka7/hop.nvim` plugin:

- All text is greyed out, with only hint letters coloured.
- Lines close to the current cursor position use single-letter hints.
- Single-letter hints are red.
- Lines farther away use two-letter hints.
- Two-letter hints are green.
- The first and second characters of a two-letter hint use different colours.
- After the first character of a two-letter hint is pressed, only hints with
  that prefix remain; they display only their second character and change to
  red.

Jump targets must be limited to locations actually visible in the current
view. In particular, words beyond the right edge of a horizontally scrolled
pane must not receive unreachable labels. Proximity therefore needs to follow
projected screen rows across soft wraps, folds, and diff filler rather than raw
document rows.
