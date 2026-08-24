---
title: "Explorer plans stay stale after unrelated child-directory activity"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: 10947e7
---

## Resolution

Commit 10947e7 (`Keep explorer plans valid during child activity`) fixes the
conflict check in `FsPlan::apply`. It was comparing the complete derived
`DirectorySnapshot` equality, including length and modification timestamps for
every child directory. Runyte's own writes below `.runyte/`, or ordinary build
activity below `target/`, therefore made an explorer of the project root stale
even when the confirmed plan did not touch those directories.

`DirectorySnapshot::matches_current` now treats child directories as the
visible path-and-kind entries shown by that explorer. `FsPlan` separately
fingerprints every existing source that the plan will rename, move, copy, or
delete. This allows unrelated child activity while retaining the safety check
for the directory being deleted and for every other operation source. Direct
changes to the explorer's visible entries still reject the entire plan before
any operation runs.

Coverage lives in `tests/fs_plan.rs`:

- `changes_inside_an_unaffected_child_directory_do_not_stale_the_plan`
- `changes_inside_a_child_directory_being_deleted_stale_the_plan`
- `a_changed_directory_conflicts_before_any_operation`

Known limitation: directory source fingerprints use the filesystem metadata
available for that directory; they do not recursively hash an entire directory
tree.

## Report

Deleting the directory `temp/` in the explorer produced a warning reporting
that the directory had changed on disk, that it should be reopened before
applying, that no operation had been applied, and that the directory edits
were retained.

Restarting Runyte and reopening the directory did not clear the condition.
