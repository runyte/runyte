---
title: "Terminal focus entered different modes depending on the navigation route"
status: resolved
reported: 2026-08-24
resolved: 2026-08-24
legacy_commit: fbcc747
---

## Resolution

Commit `fbcc747` (`Enter terminal input on pane focus`) made the focused pane's
content decide the destination mode. `App::focus_from_terminal_insert` had
special-cased directional movement from Terminal Insert by changing the
application mode to Normal before focus and forcing it back to Normal after
focus, even when the destination was another terminal. Pointer activation and
pane cycling instead returned a terminal destination to its live screen and
entered Insert, which made the same terminal behave differently according to
the input device or pane command used to reach it.

`App::finish_pane_focus` now settles directional focus, pane cycling, and the
successor focus chosen when an active pane closes at one shared boundary. A
terminal destination discards any captured review with
`TerminalSession::scroll_to_live`, enters Insert, and resets pending key
grammar; a document reached directly from Terminal Insert still enters Normal.
An unavailable directional move changes neither focus nor mode. Because
Terminal Insert continues to recognize the restricted `Ctrl-w` namespace and,
when configured, `Ctrl-h/j/k/l`, the same commands can leave the newly focused
terminal immediately without an explicit Normal transition. The separate
`Ctrl-\` dispatch was not changed and still stages Insert → live Normal →
review. Protocol version 31 prevents a new persistent client from silently
delegating this host-owned transition to an older host.

Coverage lives in `tests/terminal.rs`:
`directional_pane_keys_focus_another_terminal_in_insert` checks prefixed and
fast directional movement in all four directions, including moving away again
without pressing `i`;
`control_w_focus_moves_directly_from_terminal_insert_without_sending_input`
and `fast_pane_keys_move_out_of_terminal_input_without_reaching_the_child`
cover terminal/document boundaries and child-input isolation;
`control_w_focus_preserves_review_until_an_insert_key` covers terminal review
for directional and cycling commands;
`closing_a_document_pane_preserves_terminal_review` covers the pane-close
successor path; and
`control_backslash_steps_from_terminal_input_through_normal_to_review` pins the
unchanged staged transition.
`pointer_focus_uses_insert_for_a_live_terminal_and_preserves_review` in
`src/app/tests/presentation_and_settings.rs` retains the matching mouse
behavior.

A later behavior refinement kept that consistent destination-based boundary
while qualifying a terminal's natural mode by its own retained state.
`App::settle_terminal_focus` now starts Insert only for a live terminal; a
terminal that already owns a review snapshot keeps it and gains focus in
Normal until an explicit terminal insert key returns to live input. The same
boundary covers directional and cycling commands, pointer focus, pane-close
successor focus, and showing an existing terminal session. Protocol version 36
keeps this host-owned transition consistent for attached clients. Current
coverage lives in `tests/terminal.rs` tests
`control_w_focus_preserves_review_until_an_insert_key`,
`control_w_from_document_insert_preserves_terminal_review`,
`closing_a_document_pane_preserves_terminal_review`, and
`showing_a_reviewed_terminal_preserves_review`, plus
`pointer_focus_uses_insert_for_a_live_terminal_and_preserves_review` in
`src/app/tests/presentation_and_settings.rs`.

## Report

Moving focus to a terminal pane selected different modes depending on how the
pane was reached. `Ctrl-w` pane motions and configured direct Ctrl-based pane
motions activated Normal mode, while a mouse click activated Insert mode.

A terminal pane was expected to activate Insert mode regardless of the focus
route because Insert is its natural and most frequently used mode. Ctrl-based
motions were still expected to move directly from a terminal to another pane
without first changing to Normal. `Ctrl-\` was expected to retain its existing
Insert → Normal → review cycle, and other terminal behavior was expected to
remain unchanged.
