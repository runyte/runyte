---
title: "X retracts whole-line selections instead of extending above"
status: resolved
reported: 2026-09-28
resolved: 2026-09-29
commit: 4c8363f
---

## Resolution

4c8363f (`Add outer-edge line extension and bind X to extend above`) adds
`App::extend_line` alongside `select_line`. The older function moves a shared
head row, so changing direction can shrink the range. The new function uses
both outer bounds and grows only the requested boundary. It preserves inclusive
positions and transient whole-line semantics, unlike Helix's half-open storage.
`X` invokes `extend-line-above`; `extend-line-below` and the retained
`select-line-up` are named actions for configured bindings. Terminal review
uses equivalent boundaries.
The tutorial now uses selection undo to retract the second `x`. A bare caret
on a one-character or empty row first establishes whole-line mode; it is not
mistaken for an existing whole-line selection merely because its inclusive
endpoints coincide with the row bounds. This keeps counts consistent across
short and ordinary rows.

Regression coverage: `extend_line_above_grows_outer_edge_and_selection_undo_restores_it`,
`extend_line_commands_cover_partial_reversed_and_empty_ranges`,
`extend_line_counts_first_snap_on_empty_and_single_character_rows`, and
`keyed_and_direct_counted_line_selection_are_equivalent` in `src/app/tests/editing.rs`;
`review_line_extension_grows_both_outer_edges` and
`review_line_extension_first_snaps_a_single_character_row` in `src/terminal/mod.rs`;
`line_keys_emit_explicit_directional_range_intents` in `src/input_grammar.rs`;
`command_inventory_classifies_every_command_and_current_binding` in
`src/app/tests/commands.rs`; `every_complete_hint_description_fits_forty_four_terminal_cells`
in `src/key_hints.rs`; and `tutorial_curriculum_advances_through_the_real_input_grammar`
in `src/app/tests/tutorial.rs`.

## Report

The follow-up discussion on [issue #4](https://github.com/runyte/runyte/issues/4)
requests Helix's `extend_line_above` and `extend_line_below` commands as
configurable actions. Selection undo/redo is already implemented on `Alt-u`
and `Alt-U` (`Alt-Shift-u`), so `X` no longer needs to retract the edge walked
by `x`. The initial follow-up proposed keeping the default; the accepted
decision is to make `extend-line-above` the default `X` and retain the old
`select-line-up` action for configuration.

Reproduction: start on a middle line and press `x x X`. The existing edge walk
removes the second selected line; extension above should preserve both lines
and add the line before them. Partial ranges should first expand to whole
lines. Counts, multiple selections, empty lines and selection history must
remain supported. General action-binding configuration is a separate change.
