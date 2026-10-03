---
title: "Stale saved contents exhaust Content Finder's result budget"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: a847564
---

## Resolution

Commit `a847564` (`fix(finder): exclude open files before spending disk result limits`).

Content scanning admitted saved matches before the app removed paths owned by
open buffers. Both synchronous and background scanners now receive a captured
set of those paths and skip them before reading content or charging the result
budget. Existing public scanner wrappers retain their behavior for callers
without exclusions. Post-scan reconciliation remains necessary for documents
opened during an outstanding scan.

Coverage: `content_finder_excludes_open_disk_rows_before_the_scan_budget` in
`src/app/tests/finder_disk_budget.rs` exercises both execution paths against a
50,000-row stale disk file and checks that an unopened-file result survives
without a false limit flag. It failed before the change and passes afterward.
All 53 file-picker tests and 109 app search/picker tests passed. Root reviewed
the scanner admission and app capture boundaries independently.

## Report

Content Finder drops matches from unopened files when an earlier scanned file
is already open and its saved contents contain enough matches to exhaust the
50,000-row content budget. The scanner reads and counts those saved rows, then
the application discards them because the open buffer is authoritative. Rows
discarded at that later boundary still consume the scanner's budget.

Create `z.txt` with 50,000 lines containing `needle` and `a.txt` containing
`needle from disk`. Open `z.txt`, replace its contents with `changed in memory`
without saving, and open content Finder. The current reverse-name traversal
reads `z.txt` first. The stale saved matches consume the scanner limit and the
matching row in `a.txt` is omitted, even though no retained live result needs
that budget. The synchronous embedding path and the background scanner both
have the same ordering error.

The captured paths of authoritative open buffers must be excluded before disk
files are read or counted. Existing post-scan checks still need to protect
against buffers opened while a scan is running. The total candidate ceiling,
query revision guards, and cancellation behavior must remain unchanged.
