---
title: "Search navigation kept every match editable and the primary match was hard to distinguish"
status: resolved
reported: 2026-08-11
resolved: 2026-08-11
legacy_commit: af506ed
---

## Resolution

Commit af506ed (`Make search navigation select one match`) changed
`App::step_search`, which had been rebuilding the complete match selection and
only rotating its primary index. It now installs the next or previous match as
a single selection while retaining `SearchQuery`, including any remembered
region, so later `n` and `N` presses continue to wrap through the same results.
The initial search still selects every match, preserving the direct
search-then-edit-all workflow; navigating expresses the change to a
single-match edit without requiring `Space s c` or `,`.

Snapshot construction previously gave only the primary range's head a distinct
`PrimaryCaret` role. It then gained a `PrimarySelected` role for the rest of the
range. A later presentation revision renders that body with the theme's
light-orange `selection_primary`, renders its head with the orange Select
cursor, and hides the secondary cursor blocks while the ranges remain the exact
search results. The initial search status also identifies the primary result
and says that all matches are selected.

`src/app.rs::tests::search_prompt_repeats_and_wraps_unicode_matches` covers the
all-selected initial state, status text, single-selection cycling, and a direct
`c` edit that leaves the other match untouched.
`src/app.rs::tests::a_selection_scopes_the_search_and_confines_cycling_to_it`
covers single-result wrapping inside a remembered region.
`src/snapshot.rs::tests::a_multiselection_marks_its_complete_primary_range_separately`
covers the presentation-neutral primary range roles.
`src/ui.rs::tests::editor_caret_uses_the_theme_color_for_each_mode` covers the
primary-range TUI colour.

Known limitation: after `n` or `N` reduces the editable selection, the other
matches are not retained as passive highlights. The status line continues to
show the current result index and total.

## Report

Searching with `s`, `S`, or `/` selected every match, and `n` and `N` only
moved the primary designation through that multi-selection. The current cursor
colour was too similar to the other cursor colours, so the match from which
navigation would continue was not clear enough.

After reaching a specific match with `n` or `N`, `Space s c` was still required
before an editing command such as `c` would apply only to that match. Search was
expected to keep selecting every match initially for batch edits, while
navigating to the next or previous match should reduce the editable selection
to that one result and retain the query and scoped region for further cycling.
