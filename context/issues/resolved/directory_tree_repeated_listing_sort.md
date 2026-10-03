---
title: "Directory tree navigation repeatedly clones and sorts cached listings"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: eea17b1
---

## Resolution

Commit `eea17b1` (`perf(explorer): reuse sorted directory tree listings`).

`DirectoryTree::append_rows` cloned and sorted each expanded directory on
every row projection, although `read_directory` had already sorted the cached
entries. Projection now borrows those entries and stops when the visible-row
bound is reached. `note_applied` inserts newly created or moved entries in the
same directories-first filename order, maintaining that invariant between
background refreshes.

`applied_operations_keep_cached_tree_rows_in_navigation_order` in
`src/directory_tree/tests/mod.rs` covers creates, renames, cache ordering,
projected ordering, and first/last navigation. The Linux directory-related suite
passes all 159 tests.

## Report

Directory-tree navigation and presentation clone and sort every expanded
directory listing each time visible rows are requested. The background reader
already sorts those listings. Repeated row requests from cursor movement,
selection reconciliation, and redraw therefore duplicate path allocations and
repeat comparisons even when the filesystem has not changed.

The work increases with all expanded entries, rather than only entries visible
in the pane. A tree can retain 512 directory listings with up to 4,096 entries
each and expose up to 100,000 rows, so the extra sorting and cloning can become a
material input and redraw cost in a large workspace.

Maintain the existing directories-first, filename-sorted ordering when listings
change, including immediately published filesystem operations. Construct visible
rows by borrowing the already ordered entries. Navigation order, dotfile
visibility, and background refresh behavior must stay unchanged.
