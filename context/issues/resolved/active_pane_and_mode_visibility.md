---
title: "Active pane and editor mode were difficult to identify in a busy split layout"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: 68b4ca9
---

## Resolution

Commit `68b4ca9` (`Clarify active pane and editor mode`) made pane focus and
mode visible across the two largest stable surfaces on screen.

`Theme::inactive_background` derives a ground halfway from the theme
background toward its existing overlay background. `ui::draw_pane` uses the
ordinary background for the active pane and the intermediate ground for every
inactive pane, including borders, buffer rows, unused cells, and terminal
cells whose child-selected colour is `Default`. The overlay derivation and
overlay rendering are unchanged, so active panes, inactive panes, and overlays
remain three distinct layers in both built-in and custom themes. On light
themes the layers become progressively darker; dark themes retain their
existing opposite-direction overlay step and place the inactive pane halfway
along it.

The original resolution had `TuiTheme::mode_status_style` paint the complete
global status line with the same `cursor_normal`, `cursor_insert`, or
`cursor_select` role used by the caret. The current `draw_normal_status`
retains that cue on the leftmost mode label while keeping the rest of the row
on the ordinary theme background. All bundled themes are asserted to keep
those three roles distinct. A custom theme that omits them uses `accent`,
`error`, and `warning` respectively instead of collapsing every mode to
`accent`.

The declarative keymap now binds Insert `Ctrl-\`, its legacy `Ctrl-4`
spelling, and `Ctrl-w` to `enter-normal-mode`; `Ctrl-\` is also idempotently
bound in Normal and Select. `App::handle_key_stroke` lets those terminal Insert
keys reach the same registry rather than toggling terminal state itself.
Normal `Ctrl-\` no longer sends a byte or returns to terminal input, and the
literal `Ctrl-w` compatibility command stays Normal after sending. Because the
first Insert `Ctrl-w` is only an exit, a second Normal `Ctrl-w` begins the
ordinary window namespace. `i` remains the direct return to terminal input.

Covered by `config::tests::every_theme_orders_its_active_inactive_and_overlay_grounds`
and `config::tests::built_in_themes_use_mode_specific_cursor_colors` in
`src/config.rs`; `ui::tests::inactive_panes_use_the_ground_between_the_active_pane_and_overlays`,
`ui::tests::terminal_default_cells_follow_their_panes_ground`, and
`ui::tests::only_the_status_mode_label_follows_the_caret_color` in `src/ui.rs`;
`control_backslash_and_control_w_are_one_way_insert_exits` in
`tests/keymap.rs`; `terminal_control_w_leaves_insert_without_starting_a_window_prefix`
in `tests/key_hints.rs`; and
`opening_a_terminal_starts_in_insert_mode_and_the_exit_is_one_way` plus
`control_w_exits_then_opens_the_window_namespace_and_sends_the_literal_byte`
in `tests/terminal.rs`.

Known limitation: a terminal child that explicitly paints a non-default cell
background keeps that exact colour in an inactive pane. Rewriting a child's
palette would make its TUI semantically inaccurate; the inactive pane ground
therefore applies only where the child delegated the background to Runyte.

A later terminal-mode correction refined the `Ctrl-\` behavior described
above. It remains a one-way exit from Insert, but live terminal Normal and
review are now separate states: the first press leaves input while keeping the
terminal live and colourful, and the second captures and grays the review
snapshot. Buffer Normal remains idempotent. Coverage lives in
`tests/terminal.rs` test
`control_backslash_steps_from_terminal_input_through_normal_to_review`.

## Report

In a large window with multiple panes, especially when some panes contain
information-dense terminals, the active pane and the current NOR, INS, or SEL
mode were difficult to identify immediately.

The requested pane hierarchy was an active pane on the ordinary background,
inactive panes on a slightly dimmer background, and menu overlays on their
existing more-separated background. Inactive panes were expected to sit
between active panes and overlays rather than looking like overlays
themselves.

The global status line was expected to change with the editor mode by following
the caret colour. Every theme was expected to distinguish NOR, INS, and SEL.

`Ctrl-\` was not expected to toggle between Insert and Normal. Entering terminal
input was expected to happen through `i`, while `Ctrl-\` or `Ctrl-w` returned
to Normal.
