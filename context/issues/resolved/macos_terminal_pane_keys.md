---
title: "macOS terminal pane keys required an intermediate mode change"
status: resolved
reported: 2026-08-24
resolved: 2026-08-24
commit: 31b13b9
---

## Resolution

Commit `31b13b9` (`Disambiguate macOS terminal pane keys`) corrected the
keyboard-enhancement profile selected by `TerminalGuard::enter`. The terminal
guard had excluded macOS entirely from `PushKeyboardEnhancementFlags` because
repeat and release reports had been unreliable there. That also discarded the
independent escape-code disambiguation capability, so terminal pane Ctrl chords
did not reach the existing Insert-mode bindings with the same deterministic
identity as they did on Linux.

`keyboard_enhancement_flags_for` now gives macOS a bounded profile containing
`DISAMBIGUATE_ESCAPE_CODES` and `REPORT_ALTERNATE_KEYS`, while leaving
`REPORT_EVENT_TYPES` and `REPORT_ALL_KEYS_AS_ESCAPE_CODES` off. The first two
flags make `Ctrl-w` and `Ctrl-h/j/k/l` unambiguous in supporting terminals; the
omitted flags preserve the legacy cadence detector and avoid the repeat stream
that prompted the platform exclusion. Terminals without the protocol ignore
the request and retain Crossterm's legacy control-byte decoding. The full Linux
profile is unchanged, and the terminal guard now pops either Unix profile on
every cleanup path. Protocol version 33 rejects an older persistent client
whose physical terminal input still uses the superseded macOS profile.

Coverage lives in `src/main.rs` test
`keyboard_reporting_profiles_keep_macos_control_keys_unambiguous_without_event_types`,
which pins the macOS `CSI > 5 u` profile, the unchanged full profile, and the
absence of event-type/all-key reporting on macOS. `tests/keymap.rs` test
`control_backslash_exits_insert_and_control_w_moves_between_panes` retains the
Insert prefix contract. `tests/terminal.rs` tests
`control_w_focus_moves_directly_from_terminal_insert_without_sending_input`,
`fast_pane_keys_move_out_of_terminal_input_without_reaching_the_child`, and
`directional_pane_keys_focus_another_terminal_in_insert` cover the two
spellings, child-input isolation, review isolation, and terminal/document
destination modes.

## Report

In a terminal pane on macOS, directional pane navigation did not complete in
the first key sequence while the pane was in Insert mode.

With `editor.fast_pane_keys` enabled, each of `Ctrl-h`, `Ctrl-j`, `Ctrl-k`, and
`Ctrl-l` changed the editor from Insert to Normal on the first press. A second
press was required to move to the adjacent pane. On Linux the first press moved
immediately.

The prefixed form also differed from Linux. Pressing `Ctrl-w` first changed
Insert to Normal; pressing `h`, `j`, `k`, or `l` after it then entered terminal
review instead of moving to the adjacent pane.

Both forms were expected to execute the directional focus command immediately
from Terminal Insert. A terminal destination was expected to start live Insert,
while a document destination was expected to start Normal. Neither form was
expected to capture terminal review or send its control bytes to the child.

The correction needed to retain the macOS safeguard against unreliable
enhanced keyboard repeat and release events.
