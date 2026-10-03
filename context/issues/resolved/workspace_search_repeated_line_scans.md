---
title: "Dense workspace matches repeatedly rescan long lines"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 6684665
---

## Resolution

Commit `6684665` (`perf(search): scan match columns and previews once per line`).

`workspace_search::extend_matches` counted characters from the start of a
line for every match and rebuilt its trimmed preview each time. It now advances
byte and character positions across disjoint match fragments and prepares the
preview once per matching line. Cancellation is checked between matches,
including dense matches within one line. Unicode columns, zero-width matches,
and the existing result bound remain unchanged.

`dense_unicode_matches_keep_columns_and_share_the_line_preview`,
`zero_width_matches_keep_unicode_character_columns`, and
`cancellation_is_observed_between_matches_on_one_line` in
`src/workspace_search/tests/mod.rs` cover these behaviors. All nine module tests
pass. The dense case reduces character-prefix counting from 100,000,000
characters to 20,000 and trims its one-megabyte whitespace suffix once rather
than once per result; these are input-derived work counts, not wall-time claims.

## Report

Workspace search repeatedly scans the same text while producing several
matches from one line. For every match, `extend_matches` counts characters
from the beginning of the line to the match and trims the complete line again
to build its 240-character preview.

A long line containing many matches therefore incurs quadratic prefix-count
work. Trailing whitespace can independently cause the same suffix to be scanned
up to 10,001 times, the worker's result-limit sentinel included. This can delay
search results and cancellation even though scanning runs off the input thread.

For reproduction, search for `λ` in a line containing 10,000 `λ` characters
followed by one MiB of spaces. All matches share the same preview. Character
columns and match lengths must remain correct for Unicode and zero-width
regular expressions. Compute columns from successive match boundaries and
construct the shared line preview once, retaining the existing result cap and
snapshot format.
