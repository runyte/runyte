---
title: "Control characters in Git status paths shift action rows"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 7c3df5b
---

## Resolution

Commit `7c3df5b` (`fix(git): escape control characters in status path labels`).

`FileRow::new` interpolated raw display paths into a single-row projection.
Newlines created additional buffer rows without corresponding action identities.
It now uses the existing Git path-display escaping for both the destination and
rename source. Each file occupies one logical row while its action retains the
original operating-system path.

Coverage: `control_characters_in_paths_cannot_shift_status_action_rows` in
`src/git/view.rs` verifies physical row count, escaped labels, and raw action
paths for renamed and subsequent files. All 15 view tests passed.

## Report

The changed-file list interpolates raw path display text into each row.
Filenames containing newlines produce multiple buffer rows, while the
`status_entries` identity mapping still records one row per file. A selection
on the additional row can therefore stage, unstage, open, or prepare to discard
a different file from the one displayed. Later files can become unreachable
through their visible rows. Rename source names have the same problem.

Reproduce using a modified path named `first\ncontinuation.txt` followed by
another modified file. Build the Git status projection, join its row text
with newlines, and compare buffer row numbers with the corresponding entries.
The newline in the first path shifts visible text away from its identity.

Every status entry must occupy exactly one logical row. Escape control
characters in display names with the existing Git path-display helper while
preserving original operating-system paths for every action. Both sides of
rename labels need the same treatment; ordinary names must keep their current
presentation.
