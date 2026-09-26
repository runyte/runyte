# Color change counts in committed Git comparisons

The committed comparison opened with `Space g w`, select a worktree, then
`Tab d`, or with `Space g b`, select a branch, then `Tab d`, displays its
summary and per-file change counts in the same text color as the surrounding
content. For example, the summary can show `44 files · +3032 -810`, while a
file row shows `+34 -16`. The counts are difficult to distinguish at a glance.

Use conventional Git colors for the numeric change indicators: green for
added lines (`+N`), red for removed lines (`-N`), and yellow for a distinct
modified-line indicator if the view presents one. Apply this to both the
summary totals and the per-file counts in branch and worktree comparisons.
Color only the indicators, including their signs; leave paths, labels, and
other row text in their usual colors. The current comparison shows additions
and removals, with no separate modified-line count.
