---
title: "Git diff has no side-by-side view of complete file versions"
status: resolved
reported: 2026-08-19
resolved: 2026-08-19
legacy_commit: 2d7ee27
---

## Resolution

Commit `2d7ee27` (`Compare Git file versions side by side`) fixed
`App::open_git_diff`, which could request and open only Git's unified patch
text. The Git provider and asynchronous service had no operation that returned
the complete contents on both sides of the selected staged or unstaged change,
so the existing `DiffSession` comparison could not be used for a Git view.

The new bounded `GitProvider::file_comparison` read returns explicit text,
binary, or absent content for both versions. An unstaged comparison reads the
index and working tree; a staged row in the changed-file list reads `HEAD` and
the index. `App::open_git_file_comparison_result` places those versions in
read-only generated buffers and connects their panes with the existing
alignment model, which makes an absent side of an added or removed file render
as the same hatched filler used by `:difft`. `Space g D` and
`:git-diff-side-by-side` reach that path through the shared command and keymap
registry. Reusing `Space g D` is deliberate: the earlier discard binding at
that sequence remains removed, while this command is a read-only Git view.

Coverage is in `src/app.rs` with
`space_g_shift_d_compares_complete_index_and_worktree_versions` and
`side_by_side_git_views_make_added_and_removed_sides_empty`; in
`tests/git_provider.rs` with
`complete_file_comparisons_follow_each_diff_scope_and_preserve_absence`; and in
`tests/keymap.rs` with `git_namespace_keeps_only_navigation_and_refresh_commands`.
The complete change also passes `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`.

Known limitation: binary files are refused rather than compared as text.

## Report

Git diff (`Space g d`) showed a raw view of removed (`-`) and added (`+`)
lines. A second view was needed like the one `:difft` opens: two panes with the
previous file version on the left and the new file version on the right. For a
new file the left pane was to be empty, using the `////` markers from `:difft`;
for a removed file the right pane was to be empty with the same markers. The
new mode was to be activated with `Space g D`.
