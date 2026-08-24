---
title: "Parent explorer navigation loses the child directory selection"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: a548ba8
---

## Resolution

Commit `a548ba8` (`Restore explorer child selection in parent`) fixed parent
navigation in `App::open_parent_directory`. The function previously delegated
entirely to `open_file`, which could only restore a view already saved for the
parent directory or initialize the selection at the first row. It did not
carry the child directory being left into the parent listing, so a parent that
had not been visited before opened at the top and an older saved selection
could point at an unrelated entry.

Parent navigation now carries the child path across the retarget and selects
its row after the parent listing opens. `DirectoryReloadConfirmation` retains
that focus target when unsaved explorer edits require confirmation, so
accepting the discard has the same result as immediate navigation. The lookup
is deliberately optional: when the child is absent from the projection, such
as a dot-directory filtered by the hidden-file setting, the parent's saved or
initial view remains intact.

Coverage lives in `src/app.rs`:

- `parent_navigation_selects_the_child_without_a_saved_parent_view`
- `parent_navigation_selects_the_child_over_an_older_saved_row`
- `confirmed_parent_navigation_still_selects_the_child`
- `parent_navigation_keeps_the_fallback_view_when_the_child_is_filtered`
- `directory_navigation_retargets_one_buffer_and_preserves_each_view`
- `navigating_away_from_a_dirty_explorer_asks_before_discarding`

Known limitation: an entry filtered from the parent explorer cannot be
selected; Runyte preserves the parent's fallback view instead.

## Report

Pressing `-` in the explorer to move one directory up placed the cursor at the
top of the parent listing instead of on the directory that had just been left.
The expected traversal was:

- `-` to open the parent directory with the previous directory selected.
- Enter to immediately return to that same directory.

Keeping the child selected makes repeated traversal across directories faster.
