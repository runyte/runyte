---
title: "Scoped search includes text outside half-open selections"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: a8f2500
---

## Resolution

Commit `a8f2500` (`fix(search): preserve half-open selection scope boundaries`).

SearchRegion now retains the originating selection semantics. Both the two-character scoping threshold and subsequent region reads use half-open bounds for syntax selections, while inclusive Runyte selections retain their existing behavior. Region mapping through transactions preserves that distinction.

Coverage: expanded_syntax_search_excludes_the_half_open_boundary_in_preview_and_repeats and a_one_character_half_open_selection_does_not_scope_the_search in src/app/tests/search_region_boundaries.rs. Both failed before the fix and pass after it; they cover preview, accepted matches, Unicode edits, repeats, and the minimum selected length.

## Report

Search scoping treats every selection as an inclusive Runyte range. Syntax
expansion creates half-open selections, whose ending offset is outside the
selected text. `pane_scoping_region` and `region_spans` apply `operative_span`
without preserving that distinction, so the search includes the next character.

For example, expand the selection over `demo` in `fn demo() {}` and search for
`(`. The search preview and accepted search can match the unselected opening
parenthesis. A one-character half-open syntax selection can also be mistaken
for a two-character region instead of retaining whole-buffer search behavior.

Search preview, accepted searches, and repeats after edits should use the exact
selected character spans. Keep the existing threshold of at least two selected
characters and retain the selection's boundary semantics when storing and
mapping the search region through text changes.
