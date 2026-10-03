---
title: "Git history pages lose merge ancestry"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 8b202fc
---

## Resolution

Commit `8b202fc` (`fix(git): preserve merge ancestry across history pages`).

`GitCliProvider::log_page` restarted subsequent pages from the parents of the
last displayed commit. That discarded the traversal frontier on other merge
branches. `LogCursor` now retains the initial immutable commit tip and the
number of commits already returned. Every page traverses that same tip with
`--skip`, and its total-page count uses the same history even if HEAD advances.
Full object-ID validation and existing page/output bounds remain in place.

Coverage: `history_pages_preserve_both_sides_of_merges` in
`tests/git_provider.rs` compares every page with one complete traversal and
advances HEAD between pages. All four provider history tests and 145 app Git
tests in `src/app/tests/git.rs` passed.

## Report

Git history pagination can omit reachable commits after a merge. The first page
uses `git log HEAD`; subsequent pages restart from the parents of the last
displayed commit. That commit belongs to only one side of a merge, so restarting
there loses the traversal frontier on the other side. Pages can also repeat
shared ancestors already shown on another side.

Create a base commit, two commits on each of two branches, and a merge joining
the branches. Read the history with a page size of two and follow every returned
cursor. The resulting object IDs must equal a single `git log --topo-order
--date-order --format=%H HEAD` traversal, in order and without omissions or
duplicates. The current cursor instead drops commits from one branch.

Pagination must retain the initially captured history tip and traversal
position. Adding commits after the first page must not shift subsequent pages
or change the history being browsed. Page sizes and command output must retain
their existing bounds; cursor values are internal editor state, not a durable
or external protocol.
