---
title: "Jump labels repeatedly measure offscreen word prefixes"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 376479a
---

## Resolution

Commit `376479a` (`perf(navigation): restrict jump labels to visible word starts`).

label_visible_words now derives the unwrapped candidate column bound from the same scrolled-cell conversion used for display. Words starting past the visible right edge are rejected before their screen prefixes are measured. Wrapped and table segments keep their existing bounds, including partial label visibility, tab origins, and wide-character geometry.

Coverage: goto_word_long_unwrapped_suffix_preserves_scrolled_tab_and_wide_cell_labels in src/app/tests/jump_word_viewport.rs compares exact labels with and without 5,000 offscreen words across scrolled, tab, wide-cell, midword, and clipped-edge cases. All 15 selected goto_word tests pass. Exploratory debug runs improved roughly 418–479 ms to 0.474–0.508 ms per long-suffix case; no release-build latency claim is made.

Known limitation: candidate enumeration still performs one linear scan of the logical line. This change removes repeated prefix measurements for offscreen words rather than eliminating all work proportional to line length.

## Report

`App::label_visible_words` scans a logical line and measures each eligible word's
screen column. In an unwrapped pane its candidate column range is
`scroll_col..usize::MAX`, so every word after the horizontal scroll origin
reaches `cells_from_column`, including words far beyond the pane's right edge.
That helper walks the line prefix to the candidate. Repeating it for all words
makes jump-label activation quadratic on a long line containing many words,
even though only the small visible prefix can receive labels.

Reproduce with soft wrapping disabled and a long line made from repeated `aa `,
then invoke `goto-word` while the viewport is at the start of the line. Increasing
the offscreen suffix increases the repeated prefix measurements; the set of
visible labels stays the same.

Restrict candidate starts to the viewport before measuring each candidate's
screen position. Preserve tab and wide-character geometry, horizontally scrolled
panes, and a word whose first label cell is visible while its second lies just
outside the viewport. A whole-line scan may remain linear, but offscreen words
must not each trigger another prefix scan. Wrapped and rendered-table projections
must continue to use their existing segment ownership rules.
