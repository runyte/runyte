---
title: "Rotating the primary after a search draws every match's head as a cursor"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: 151dea2
---

## Resolution

Commit `151dea2` (`Keep a search result drawn as matches while the primary
rotates`) fixed `App::rotate_selection` in `src/app.rs`.

A committed search records a `SearchSelectionPresentation` pinned to the
pane's `selection_revision`. While that presentation is pristine the
secondary carets are suppressed, so the result reads as a set of matches with
one of them leading rather than as twenty-five cursors.
`reconcile_search_selection_presentation` drops it as soon as the revision
moves, on the correct principle that a motion has turned the matches into
ordinary ranges. `(` and `)` were caught by that rule for the wrong reason:
`Selection::rotate` clones the ranges untouched and only moves the primary
index, but it still goes through `Pane::replace_selection`, which bumps the
revision. The presentation was therefore let go, every match's head began
drawing as `TextRole::Caret`, and in Select mode `Caret` and `PrimaryCaret`
both resolve to `theme.cursor_select` in `src/ui.rs`, so every head took on
the primary cursor's colour. The selections stayed correct throughout,
because `Selected` and `PrimarySelected` are distinct colours.

`rotate_selection` now reads whether the presentation was pristine before the
replacement and re-stamps it onto the new revision if it was, which is the
narrow statement that rotating chooses which range leads without changing
what the ranges are. Everything else that changes a selection still lets the
presentation go. The status line was left standing at `match 1/25` over a
different match, so the rotation now reports the new primary in the wording
the search itself uses.

Covered by `rotating_the_primary_keeps_a_search_result_drawn_as_matches` in
`src/snapshot.rs`, which asserts the whole run sequence of the row, alongside
`pristine_search_hides_secondary_carets_until_the_selection_moves` for the
case where the presentation should still be dropped.

Known limitation: an ordinary caret and the primary caret remain the same
colour in Select mode, so in any multi-selection that is not a pristine
search result the primary is told apart by its selection background alone.
Giving the secondary caret its own colour is a theme question across all
bundled palettes and was not decided here.

## Report

After `s at Enter`, all matching `at` strings were shown with the primary one
highlighted with the primary cursor and the primary selection colours.
Pressing `(` or `)` then drew every match's cursor in the primary cursor
colour. The selections continued to distinguish the primary range from the
secondary ones correctly.
