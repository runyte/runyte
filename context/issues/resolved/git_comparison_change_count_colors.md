---
title: "Committed Git comparison change counts lack distinct colors"
status: resolved
reported: 2026-09-26
resolved: 2026-09-26
commit: 0fc6d96
---

## Resolution

Commit `0fc6d96` (`Color committed Git comparison line counts`) fixes the
missing count metadata in committed comparison projections.
`RevisionComparison::render` generated only text, and the snapshot path
looked up `CountColumns` only for the Git status list. Comparison totals and
file counts therefore reached the frontend as ordinary text.

`RevisionComparison::render_with_counts` now records the signed addition and
removal ranges while generating each row. These are character columns, so
Unicode paths and display-width padding cannot displace the highlights, and
count-like text in paths or labels is never interpreted as a numeric indicator.
The comparison document retains the ranges alongside its captured revisions
and regenerates them with the text on refresh and resize. Snapshots publish
the existing `CountKind` values, which both bundled frontends already draw
using `change_added` and `change_removed`. Built-in themes use green and red;
custom themes retain control of those roles. Binary and metadata labels stay
ordinary text. No separate modified-line count is introduced.

Validation passed with `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` (4,034 passed,
36 ignored). The canonical Linux `cargo llvm-cov --locked --workspace` run
passed at 92.06% line coverage, above the unchanged 89% floor. Full-suite runs
used unrestricted local process and socket access after sandbox permission
errors in unrelated fixtures.

Regression coverage:

- `committed_comparison_count_colors_follow_branch_and_worktree_refresh` in
  `src/app/tests/git_comparison.rs` checks both entry points, rendered green/red
  signs and digits, summary and file counts, resize, and refreshed totals.
- `committed_comparison_count_ranges_exclude_paths_labels_and_metadata` in
  `src/app/tests/git_comparison.rs` checks Unicode renames, count-like paths
  and labels, multi-digit and zero counts, binary and metadata-only rows,
  empty files, and refresh to an empty comparison.
- `count_runs_use_the_theme_change_colours` in `src/ui.rs` covers the shared
  renderer's use of the theme's addition/removal colors and ordinary text
  color outside those ranges.

## Report

The committed comparison opened with `Space g w`, select a worktree, then
`Tab d`, or with `Space g b`, select a branch, then `Tab d`, displays its
summary and per-file change counts in the same text color as the surrounding
content. For example, the summary can show `44 files · +3032 -810`, while a
file row shows `+34 -16`. The counts are difficult to distinguish at a glance.

The expected numeric change indicators use conventional Git colors: green for
added lines (`+N`), red for removed lines (`-N`), and yellow for a distinct
modified-line indicator if the view presents one. This applies to both the
summary totals and the per-file counts in branch and worktree comparisons.
Only the indicators, including their signs, should receive these colors;
paths, labels, and other row text retain their usual colors. The comparison
shows additions and removals, with no separate modified-line count.
