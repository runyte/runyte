---
title: "Opening the explorer from a file starts at the top of the directory"
status: resolved
reported: 2026-08-21
resolved: 2026-08-21
legacy_commit: 2f2c12f
---

## Resolution

Commit `2f2c12f` (`Focus active file in explorer`) fixed active-file explorer
navigation in `App::open_active_directory_explorer`. The function previously
computed the active buffer's directory and delegated directly to
`open_explorer`, which restored an older saved directory view when one existed
and otherwise initialized the explorer selection at the first row. It did not
carry the file that supplied that directory into the listing.

The command now captures the active file path before opening its parent and
uses the explorer's existing path-based entry focus after navigation succeeds.
If a pane-owned explorer for another directory has unsaved edits, the path is
instead retained in `DirectoryReloadConfirmation`, so accepting the discard
selects the same file once the deferred navigation completes. Enter then uses
the ordinary directory-entry path and returns to the already-open file buffer.

Coverage lives in `src/app.rs`:

- `active_directory_explorer_selects_the_file_it_was_opened_from`
- `confirmed_active_directory_explorer_still_selects_the_file`

Known limitation: a file filtered out of the explorer by the hidden-file
setting cannot be selected; the explorer keeps its saved or initial view.

## Report

Opening the explorer with `Space e` from an active file buffer placed the
cursor at the top of the directory instead of on the entry for that file. The
expected sequence was:

```text
Space e
Enter
```

This should return to the same file buffer.
