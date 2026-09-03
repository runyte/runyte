---
title: "A finder content match cannot be previewed after a truncated scan is restarted"
status: resolved
reported: 2026-09-01
resolved: 2026-09-01
commit: 2c8eba1
---

## Resolution

Fixed by 2c8eba1 "Re-rank a truncated content scan under its restart query".

The rows on screen and the entries behind them belonged to two different
scans. A content scan keeps only the lines its own query matched, so a scan
that stopped at `CONTENT_ENTRY_LIMIT` cannot answer a longer query: the
project holds matches it never reached. `FilePicker::content_rescan_needed`
recognises that and `restart_content_scan_if_needed` restarts the scan under
the current query, which is correct and was not what went wrong.

What went wrong is the handoff to the background ranker. `start_content_scan`
calls `FileScanner::scan_content`, which calls `reset_ranker` for the new scan
id, and the reset replaces the worker's `FileRankState` wholesale: the query
it was answering, that query's revision, and the finder's file-rank context —
the finder's live buffer and terminal matches and its suppressed paths — all
go with the entry table they described. Restating them was left to the caller.
`start_content_scan` re-requested a rank only when `self.finder.is_none()`,
and the finder path relied on the `rank_resource_finder` call that
`handle_picker_key` makes after every query edit.

The path that restarts from a `Finished` event has no keystroke behind it. It
runs inside `apply_file_picker_event` when a scan reports `limited`, and it
restated nothing. The ranker therefore kept ranking the refilled entries
against the empty query at revision zero, and the picker's own
`query_revision` guard discarded every result. The settled state was
`entries` full, `matches` empty, `preview` cleared by the restart, `ranking`
and `loading` both false so the overlay read `ready` — and `finder.matches`
still holding the previous scan's rows, because a discarded `Ranked` event
leaves the finder untouched.

Those rows still drew, since `picker.view(entry)` resolves any index the
rebuilt table happens to cover, but `refresh_finder_preview` looks the
selected file match up in `picker.matches` before asking for a preview and
found nothing there. Enter would have been worse than the missing preview: it
resolves the same stale index and would have opened whichever file now sat at
that slot.

The fix moves the restatement into `restart_content_scan_if_needed`, which is
the funnel every content-query restart passes through, rather than adding a
second copy to the `Finished` handler. Both restart paths now hand the ranker
the current query, its revision, and a full finder context. Taking that
context through `rank_resource_finder` also restarts the in-memory resource
content scan, which is required rather than incidental:
`take_file_rank_context` moves the finder's `resource_matches` out of the
finder and into the ranker, so once the ranker has been reset those matches
exist nowhere and a delta context would publish an empty resource list.
Restarting the resource scan republishes them as a replacement, and it clears
the stale rows in the same step, so the list rebuilds instead of lingering
with dead indices. The query-edit path now states this twice, which costs one
superseded rank request and no scanning work, since the resource scan
advances in bounded slices from the event loop.

A later change hardened the boundary the report exposed rather than only the
path that crossed it. An entry index is meaningless without the scan that
produced it, so `ResourceFinder` now records which picker scan its file
matches were ranked against, and `ResourceFinder::file_entry` is the single
way one of those matches becomes an entry view. Rows, previews, and what
`Enter` opens all resolve through it, so a match ranked against a table the
picker has since rebuilt resolves to nothing instead of silently naming an
unrelated line. That is a guard rather than a second fix: with the
restatement above in place, no path is expected to reach it.

A second preview path still discarded that scan identity after using the
guard. `refresh_finder_preview` first proved that the selected finder row was
readable through `ResourceFinder::file_entry`, but then searched for the same
bare entry index in the new picker's match list and delegated previewing to
that row. During rapid query edits or `Tab` mode switches, a content re-scan
legitimately leaves the displayed row in the retained corpus while the new
corpus has no match at that index. The row could therefore be drawn from its
original scan while its preview stayed empty; a reused index could also have
selected an unrelated new row. Finder previews now derive the path, row, and
match emphasis directly from the scan-aware entry view and pass that complete
selection to the shared file-preview loader.

Tests: `truncated_content_rescan_reranks_under_the_query_it_restarts_for` in
`src/app/tests/search_and_pickers.rs` is the regression test, with
`a_restarted_scan_retires_the_file_matches_ranked_against_its_entries` in
`src/finder.rs` covering the guard directly — it asserts that the rebuilt
table does answer the stale index, which is the hazard, and that the finder
refuses to read it there anyway. It drives the
real background scanner, types a query the initial scan did not run under,
injects the truncated `Finished` that triggers the restart, and requires the
restarted scan to be ranked, every finder row to resolve against the rebuilt
entry table, and the selection to be previewable. Without the fix it never
settles. `a_retained_content_row_keeps_its_preview_while_the_new_scan_ranks`
and `rapid_finder_mode_switches_and_retyped_query_restore_the_file_preview`,
also in `src/app/tests/search_and_pickers.rs`, cover the retained-corpus
preview boundary and the reported background-scanner interaction sequence.

## Report

The project finder in content mode sometimes shows `No preview` for a
selected match, and stays that way for every row in the list rather than for
one file.

Observed in a workspace rooted at a home directory holding many files. The
finder header read:

```
Find · Contents · ready · 18157/50000 · result limit reached
```

with the query `Test`, a full list of file content matches drawn, the first
row selected, and the preview pane showing `No preview`.

Expected behavior is that a drawn content match is previewable: the preview
pane shows the matching line in its surrounding context, with the matched
text emphasised, exactly as it does when the result limit is not reached.

The reproduction is a query typed faster than the scan behind it completes,
in a project large enough for the scan to stop at the result limit: type one
character, keep typing while the first scan is still walking the project, and
let the scan report that it was truncated after the query has moved on.
