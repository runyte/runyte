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
