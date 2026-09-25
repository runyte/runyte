---
title: "Git log rows show the author date without a time of day"
status: resolved
reported: 2026-09-25
resolved: 2026-09-25
commit: ca79bbd
---

## Resolution

Commit `ca79bbd` (`Show commit author time in Git log rows`) fixed the
history-row presentation. `git_log_view` previously printed only the `%as`
author date, so commits made on the same day had no visible time. The Git
requests now ask Git to format the author's date and time in the commit's own
timezone; `CommitSummary` stores that bounded ASCII value and the row prints
it as `YYYY-MM-DD HH:MM`. This avoids a UTC conversion shifting commits near
midnight onto the wrong date. The date-only page heading, commit detail and
blame view remain unchanged.

Coverage is in `src/git/history.rs` and `tests/git_parsing.rs` for valid and
malformed timestamp parsing, `tests/git_provider.rs` in
`log_rows_keep_author_clock_time_across_utc_midnight` for a real commit whose
author date and time differ from the UTC/committer date, and
`src/app/tests/git.rs` for row and paging presentation. All 23 Git parsing
tests and the real-Git timezone test passed.

## Report

`Space g l` opens paged commit history. Each row shows the short object ID,
the author date as `YYYY-MM-DD`, the author, and the subject:

```text
d7009a1  2026-09-23  Author Name  Remove issue: Fixed in newest Codex
```

Two commits made on the same day cannot be told apart by time, although Git
records it. A commit's author and committer lines each store Unix seconds and
the author's timezone offset, for example `1790162700 +0200`, so the time of
day is always available.

The log request in `src/git/cli.rs` already asks for `%at` (stored as
`CommitSummary::author_time`) and `%as` (stored as
`CommitSummary::author_date`, see `src/git/history.rs`). The row format in
`src/app/git_workflows.rs` prints only `author_date`.

## Expected behavior

Each log row shows the author date and time as `YYYY-MM-DD HH:MM`:

```text
d7009a1  2026-09-23 14:05  Author Name  Remove issue: Fixed in newest Codex
```

- Show the time in the commit's own timezone, as `git log` does by default,
  not converted to UTC or the local zone. The existing date is taken from
  `%as` for the same reason: deriving it from `author_time` in UTC shifts
  some commits by a day. The time must follow the same rule. One approach is
  to have Git format it, for example `%ad` with
  `--date=format:%Y-%m-%d %H:%M` or `%ai`, instead of formatting
  `author_time` in Rust.
- Keep using the author date, not the committer date.
- The page heading (`git_log_page_heading`) keeps its date-only
  `oldest - newest` span. The user guide requires the heading to fit within
  80 characters.
- Rows stay ASCII-only, so byte and terminal-column widths stay equal.

## Scope

Undecided: whether the commit detail view opened with Enter (which also
prints `author_date`) and the blame view's date column (`src/git/blame.rs`)
should also show the time. Leave both unchanged unless consistency needs the
change. If either is changed, say so in the resolution.

## Coverage

- Update the parsing tests in `src/git/history.rs` and
  `tests/git_parsing.rs` for the new field shape. Keep a test showing that a
  commit near midnight in a non-UTC zone keeps its own local date and time.
- Update the log row fixtures in `src/app/tests/git.rs`.
- Update the `Space g l` description in `docs/user-guide.md`, which currently
  says rows show "the author date as `YYYY-MM-DD`".
