---
title: "Ctrl-\\ did not switch an integrated terminal to Normal or review on Windows"
status: resolved
reported: 2026-09-21
resolved: 2026-09-23
commit: eff117c
---

## Resolution

Commit `eff117c` (`Decode native Windows Ctrl-backslash for terminal modes`)
corrected `Decoder::native_key` in `src/tui/windows_input.rs`. A real ConPTY
control-byte input arrived as a native key record with Unicode unit `0x1c` and
Control set. The decoder normalized Ctrl+A through Ctrl+Z, but left this
punctuation control unit as `Char('\x1c')`. The terminal input dispatch recognizes
the `Ctrl-\` and legacy `Ctrl-4` spellings, so it forwarded that unrecognized
stroke to the child instead of changing editor mode.

The decoder now maps that exact native control unit to `Ctrl-\`, as its VT byte
path already did. It leaves adjacent control units and literal paste payloads
alone. The first press enters live Normal, the second enters terminal review,
and `i` returns to terminal input. Commit `1159418` (`Cover Windows terminal
review after second Ctrl-backslash`) added the second-press native acceptance.

Coverage lives in `src/tui/windows_input/tests.rs`:
`native_ctrl_backslash_unit_keeps_terminal_mode_switch_identity` checks native
and VT decoding, adjacent control input, and literal paste;
`src/tui/windows_console_acceptance.rs::console_control_key_transport` verifies
the real ConPTY input boundary; and
`tests/windows_terminal_mode.rs::native_ctrl_backslash_switches_live_terminal_modes`
exercises both presses and return to Insert in a real editor process.
`tests/terminal.rs::control_backslash_steps_from_terminal_input_through_normal_to_review`
covers the editor's semantic transition.

Known limitation: the native acceptance injects the control byte into ConPTY.
A physical keyboard, outer terminal, and keyboard layout have not been verified.

## Report

On Windows, pressing `Ctrl-\` in an integrated terminal did not switch from
terminal Insert to live Normal mode or reach Normal/review mode. The child
program and any effect it received from the key had not been recorded.

To reproduce, open an integrated terminal with `:terminal` on Windows, focus
its pane in Insert mode, and press `Ctrl-\`. Press it a second time to test the
review transition. The expected behavior is for the first press to enter live
Normal mode without freezing child output, and for the second press to capture
the terminal's review snapshot. `i` should return to terminal input.

The Windows version, outer terminal, shell, keyboard layout, and whether
Runyte received `Ctrl-\`, its legacy `Ctrl-4` spelling, or another input event
were not established by the original report. The native console input report
and editor key dispatch needed inspection before attributing the failure to
either layer.
