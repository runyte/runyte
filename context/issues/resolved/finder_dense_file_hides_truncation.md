---
title: "Dense matching files conceal Finder result truncation"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 35ff39e
---

## Resolution

Commit `35ff39e` (`fix(finder): detect overflow inside dense matching files`).

Disk content scans now collect one overflow candidate before enforcing the shared 50,000-row budget. This distinguishes a complete exactly-at-limit file from a truncated file and marks the latter limited, allowing query refinement to rescan its omitted suffix. The public line_hits helper retains its existing capped result size, and neither synchronous nor background scans publish the sentinel.

An independent follow-up found the same hidden-overflow condition in directory-scoped grep's live-buffer fallback. start_content_scan now collects at most the remaining shared budget plus one match from each open buffer. It stops only after observing that extra match, including when an earlier buffer filled the budget exactly. FilePicker admission discards the sentinel and records the limit, so narrowing the query searches authoritative unsaved text again. The shared bounded helper remains internal; the public line_hits limit is unchanged.

Coverage: a_dense_file_reports_overflow_before_narrowed_query_rescans in tests/finder_content_limits.rs reproduces the missing limited flag at 50,001 matches, checks the exact-limit case, preserves the public helper cap, finds the omitted row after narrowing, and verifies background publication stays at 50,000 with limited set. It failed before the fix and passes afterward; all 53 selected file-picker unit tests also pass.

directory_grep_marks_overflow_within_one_live_buffer and directory_grep_marks_overflow_after_an_exact_limit_live_buffer in src/app/tests/finder_disk_budget.rs both failed before the follow-up. They check an initially complete exact-limit buffer, then add an unsaved matching row in either the same buffer or a later open buffer, type a narrower query through normal key handling, advance the rescan deadline, and verify the previously omitted row's path and source coordinate.

## Report

The content scanner uses `line_hits`, which stops after exactly 50,000
matching lines in one file. The scanner subsequently truncates that already
capped list to its 50,000-row budget, so it detects no overflow and reports
`limited: false` when a single file contains additional matches. An extended
query then narrows the incomplete corpus instead of restarting the scan and
cannot find a matching line that was omitted from its end.

Create one file containing 50,000 lines `needle` followed by
`needle rare`. Search Finder contents for `needle`, then extend the query to
`needle rare`. The final row should be found by a new scan. The broad scan
must report that its results were truncated, while exactly 50,000 matches
must still be a complete scan.

Read one overflow candidate before enforcing the shared result budget, without
publishing more than 50,000 rows. Preserve the public bounded `line_hits`
helper's existing result size. Cover synchronous and background scans,
exact-budget input and a narrowed query reaching the omitted row.

The directory-scoped `:fuzzy-grep-directory` command also searches open file
buffers from their live text before scanning unopened files. Its live-buffer
collection calls the same capped public helper, so 50,000 `needle` rows followed
by an unsaved `needle rare` row produce 50,000 candidates with no limit marker.
Typing `rare` narrows those candidates to zero instead of rescanning for the
final row. The same loss occurs when the first open buffer has exactly 50,000
matches and a later open buffer contains `needle rare`: collection stops before
admitting evidence of the extra match. Both cases should expose truncation,
retain at most 50,000 published rows, and find the unsaved match after narrowing.
