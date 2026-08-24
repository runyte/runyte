---
title: "The Git blame view has no column showing commit dates"
status: resolved
reported: 2026-08-18
resolved: 2026-08-18
legacy_commit: 984f279
---

## Resolution

Commit `984f279` (`Add an author-date column to the Git blame view`) added the
missing date. `git blame --line-porcelain`, parsed by
`git::blame::parse_blame`, has no preformatted date field the way `git log`
does through `%as` — it only emits `author-time` (Unix seconds) and
`author-tz` (an offset like `+0200`), and `BlameLine` previously kept the
former and discarded the latter. `parse_blame` now also captures
`author-tz` and computes a new `BlameLine::author_date: Option<String>` from
the pair, formatting it as `YYYY-MM-DD` in the commit's own timezone rather
than the local machine's or naive UTC — the same convention documented on
`git::history::CommitSummary::author_date`, so a blame row's date reads the
same as `git log --date=short` instead of shifting by a day near a
timezone boundary. The field is `Option` because a record without a
parseable `author-time`/`author-tz` pair (synthetic or malformed input) has
no date to show. `App::open_git_blame_result` renders the date as its own
fixed-width column between the abbreviated object id and the author name.

Covered by `git::blame::tests::author_date_uses_the_commits_own_timezone`
and the no-`author-tz`-header case in
`git::blame::tests::parses_committed_and_live_uncommitted_lines` in
`src/git/blame.rs`, plus the existing blame-buffer tests in `src/app.rs`
(`log_selection_is_object_stable_and_stale_blame_is_discarded` and
`blame_refuses_oversized_and_binary_buffers_before_service_submission`),
updated for the new `BlameLine` field.

## Report

Git blame view (Space g B) should also contain a column with dates
in YYYY-MM-DD format.
