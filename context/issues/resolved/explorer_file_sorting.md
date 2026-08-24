---
title: "Explorer entries are re-sorted immediately after applying edits"
status: resolved
reported: 2026-08-13
resolved: 2026-08-14
legacy_commit: 93c0b58
---

## Resolution

Commit 93c0b58 (`Preserve explorer order after writes`) fixed the refresh path
in `App::reconcile_applied_filesystem`. It was sending the explorer that
applied a successful filesystem plan through `Buffer::reload_directory`, which
discarded the edited row order and rebuilt the sorted projection immediately.

The initiating explorer now accepts the new on-disk snapshot through
`Buffer::accept_directory_plan` and
`DirectoryBuffer::refresh_baseline_preserving_order`. That refresh maps the
surviving immediate-child rows to their new filesystem identities while
retaining their edited order, and leaves other affected clean explorers on the
ordinary reload path. Re-entering a clean explorer from a file reloads its
canonical sorted projection, while the existing dirty-explorer protection
still prevents unsaved edits from being discarded.

The behavior is covered by
`tests/directory_buffer.rs::applying_a_plan_keeps_the_edited_order_until_the_explorer_is_reentered`.

## Report

The explorer automatically sorted all entries after a newly added or renamed
file or directory was saved. This moved the entry away from the row where it
had been edited and forced the user to find it again.

The desired lifecycle was to sort when entering a directory, retain the
current row order after adding or renaming entries and applying the changes,
then sort again after leaving for another directory or file and returning to
the explorer.
