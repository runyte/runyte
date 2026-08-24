---
title: "Explorer navigation leaves selection mode active"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: d4e231a
---

## Resolution

Commit `d4e231a` ("Exit selection mode after explorer navigation") makes the
explorer navigation boundary explicitly enter Normal mode after it successfully
opens an entry or a parent directory. Previously `open_directory_entry` and
`open_parent_directory` replaced the directory/file buffer while retaining the
old selection mode. Dirty-directory confirmations now make the same transition
when the confirmed navigation resumes; invalid rows and external-program
prompts do not discard mode state prematurely.

`explorer_navigation_returns_to_normal_mode_after_selected_entries` in
`src/app.rs` covers a search-created selection entering a directory, manually
selected parent navigation, and opening a file.

## Report

In the explorer, selecting text or searching with `/` and then pressing Enter
to enter a directory or open a file left selection mode active. The mode
should return to Normal after entering a directory, moving to the parent
directory, or opening a file.
