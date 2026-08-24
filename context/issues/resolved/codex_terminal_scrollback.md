---
title: "Codex responses disappear from integrated-terminal scrollback"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: d3de7f4
---

## Resolution

Commit `d3de7f4` (`Keep inline terminal output in scrollback`) corrected
`Grid::scroll_up`, which retained a displaced line only when the active
scrolling region covered the entire screen. Codex's inline TUI instead scrolls
a top-anchored region so completed responses leave the screen while its
composer and status rows remain fixed below. Runyte moved those rows on the
live grid but discarded them instead of retiring them into bounded history,
so Terminal Normal motions, mouse scrollback, immutable review, and
`:terminal-output` all had no previous responses to read.

Top-anchored regions now retire their displaced lines because those lines
leave the terminal at row zero just like an ordinary full-screen scroll. A
region whose top margin begins below row zero still keeps no history: such a
region rearranges an application's internal pager or status area, and adding
those rows would interleave unrelated repaint fragments into the transcript.
The alternate screen also continues to keep no history.

Tests covering the behavior are
`scrolling_a_top_anchored_region_keeps_history_and_a_lower_region_does_not` in
`src/terminal/grid.rs` and
`inline_tui_output_scrolled_above_a_fixed_composer_is_reviewable` in
`src/terminal/mod.rs`. The latter drives the parser with the same DECSTBM and
scroll-up sequence used by an inline Ratatui viewport, then checks the live
scroll view, plain output used by `Space t y`, and immutable review snapshot.

Known limitation: a Codex configuration that explicitly uses the alternate
screen exposes only its current visible screen, matching terminal semantics.
Codex must run in inline mode for completed responses to become terminal
scrollback.

## Report

When Codex runs inside a Runyte integrated terminal, its previous responses
cannot be reached with the mouse or with motion keys in Terminal Normal mode.
`Space t y` also omits the previous responses. Other programs such as Claude
retain scrollable output, as do ordinary terminal sessions that print enough
commands such as `ls`.
