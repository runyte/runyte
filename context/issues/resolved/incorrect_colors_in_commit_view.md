---
title: "Commit-message bullet lines are colored as patch removals"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: 75534bc
---

## Resolution

Commit 75534bc (`Keep commit messages out of diff coloring`) fixes the commit
detail presentation boundary. `App::open_git_commit_detail_result` previously
put the metadata, full commit message, and patch into one virtual buffer marked
entirely as a diff. `snapshot::row_diff` consequently passed every row to the
unified-diff classifier, which correctly interpreted a commit-message line
beginning with `-` as a removed line because the buffer model gave it no way to
distinguish prose from patch content.

Virtual diff buffers now carry the character offset at which their patch
content begins. Whole-patch views use offset zero, while commit detail records
the exact offset before appending the structured patch value already returned
by the Git provider. Snapshot generation classifies only rows at or after that
offset, so commit metadata and message text retain ordinary coloring without
parsing the displayed prose for a delimiter. An empty message or empty patch
uses the same boundary and requires no special textual sentinel.

Tests covering the behavior are:

- `commit_detail_colours_only_rows_inside_its_patch_region` in `src/app.rs`
- `commit_detail_diff_boundary_handles_empty_message_and_empty_patch` in
  `src/app.rs`

## Report

The following commit was displayed in Runyte:

```text
commit bdf63a7bea5eff7ac84b96d7feb6c341f80a6035
Author: Krzysztof Arendt
Author-time: 1786687978
Parents: 3d5f19bd45808db9713543cc2a54d7b8f356486b

Add new issues

- catpuccin theme
- everforest theme
- fox theme
- git worktrees in space g b

...
```

The bullet list was colored red even though it was part of the commit message,
not part of the commit's actual changes.
