---
title: "gw: label on-screen words and jump to one"
status: resolved
reported: 2026-07-30
resolved: 2026-07-31
legacy_commit: c2e5c22
---

## Resolution

Implemented in commit `c2e5c22`, "Add gw jump labels for on-screen words".

What was added:

- `src/jump_labels.rs`, a new module owning label assignment and the two-key
  narrowing. It knows nothing about drawing, nor about what makes an offset
  worth labelling: the editor hands it target offsets in document order, and
  rendering asks which label character, if any, sits at an offset.
- `EditorCommand::GotoWord`, bound to `g w` in `src/keymap.rs`.
- `App::label_visible_words` collects the start of every word whose first two
  characters are two single-cell characters, within the columns the active
  pane actually draws. Two characters, because a label occupies two cells and
  must never spill past the word it names. Single-cell, because a label
  character replaces the character underneath it, so covering a double-width
  character would pull the rest of the row leftwards while the labels were up.
  With soft wrap on, both the visible rows and the visible columns of each row
  come from `wrap::visible_rows`, at the width the pane records from the last
  frame — measuring the pane without its line-number gutter would wrap at the
  wrong width and label words below the fold.
- Labels are drawn over the text in `src/ui.rs`, replacing the first two cells
  of the word, so the line keeps its width and nothing to the right shifts.
- Labels are handed out home row first (`asdfghjkl`, then the rest). The first
  keystroke narrows and hides the labels it ruled out; the second jumps,
  extending the selection in Select mode and recording a jumplist entry as any
  long move does. Any other key spends the labels.

Colours: `ThemeDefinition` and `Theme` gained `jump_label_primary` and
`jump_label_secondary`, set per built-in theme in `src/config.rs` — light blue
over dimmed blue for `base16` and `paper`, gruvbox's own bright and plain blue
for `gruvbox`. A theme that omits them falls back to the `base16` values, as
every other theme colour does. Documented in `README.md` and
`config.example.yaml`.

A later visual review found that the secondary character was too far from the
primary character in visual weight and could be difficult to read. The dark
palettes now bring the secondary colour closer to the brighter primary colour,
while the light and paper palettes use a darker pink primary and nearby lighter
pink secondary in place of blue. Both characters in every built-in palette now
meet a 4.5:1 contrast floor against the editor background, and their mutual
luminance contrast is capped so the secondary cannot become faint again.

Covered by the `jump_labels` module tests, the `goto_word_*` tests in
`src/app.rs`, and `jump_labels_paint_over_the_words_they_name`,
`jump_labels_stop_at_the_bottom_of_a_soft_wrapped_pane`, and
`jump_labels_wrap_at_the_width_the_gutter_leaves` in `src/ui.rs`, plus
`src/config.rs::tests::built_in_jump_labels_are_red_and_one_neon_cyan_hue`,
which checks background contrast and visual-weight distance.

A later Hop-style refinement removed the original hard-wrap limitation:
candidate columns are now measured in terminal cells from the visible origin,
and label assignment rejects and regenerates any label that would cross the
right edge.

A later palette refinement gives both two-key characters the same neon-cyan
hue in every built-in theme. The secondary is darker on dark backgrounds and
lighter on light backgrounds, so its relationship to the primary reverses
without appearing to change colour.

A further light-theme refinement separates temporary jump dimming from the
general `muted` role. `jump_text_muted` falls back to `muted` for older themes;
the built-in `light` and `paper` themes use lighter grays so cyan and red hints
stand farther forward without changing comments or ordinary UI text.

## Report

A two-character jump-label motion was requested, equivalent to Helix's `gw`:
pressing `g` then `w` labels every word on screen longer than one character
with a two-character sequence, and typing a sequence moves the cursor to that
location.

The labels should be coloured distinctly, with the first character bright —
light blue, for instance — and the second dimmer, adjusted per theme.
