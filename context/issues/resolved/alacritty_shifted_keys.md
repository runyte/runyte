---
title: "Shift-colon opened the semicolon command instead of the command palette in direct Alacritty sessions"
status: resolved
reported: 2026-08-21
resolved: 2026-08-21
legacy_commit: 9fa1907
---

## Resolution

Commit `9fa1907` (`Preserve shifted keys in enhanced terminals`) corrected the
enhanced keyboard flags requested by `TerminalGuard::enter`. Runyte had asked
Alacritty to report every key as an escape sequence and to report event types,
but had not requested alternate keycodes. Under that protocol configuration,
Alacritty represented `Shift-;` with the unshifted `;` codepoint plus a Shift
modifier. Crossterm therefore had no shifted codepoint from which to recover
`:`, and Runyte's binding canonicalization correctly removed the redundant
Shift modifier from what appeared to be a `;` character. The resulting stroke
ran `collapse-selection` instead of opening the command palette.

`keyboard_enhancement_flags` now requests alternate keycodes alongside event
types and all-key escape sequences. Supporting terminals consequently include
the layout-produced shifted character, which Crossterm exposes as `:` while
retaining the repeat and release events used for held-motion acceleration.
This fixes shifted punctuation and letters through the same layout-aware
protocol rather than encoding a US-keyboard punctuation table in Runyte.

Coverage lives in `src/main.rs`:
`tests::enhanced_keyboard_reporting_requests_shifted_keycodes` asserts the
complete flag set and its exact `CSI > 14 u` request. The existing
`src/app.rs::tests::command_prompt_filters_and_completes_commands` test covers
the resulting `:` command-palette behavior after the frontend input boundary.

## Report

In Alacritty without tmux, typing `:` did not enter a command. The same key
worked in Alacritty with tmux and in GNOME's default terminal. The expected
behavior was for `:` to open Runyte's command palette in all three environments.
