---
title: "There was no centered, editable writing view"
status: resolved
reported: 2026-08-16
resolved: 2026-08-16
legacy_commit: f078ab1
---

## Resolution

Commit `f078ab1` (`Add editable zen writing mode`) added `:zen` as a typed,
discoverable toggle. `App::prepare_view` previously always projected the full
split tree, while `Buffer::content_layout` could center only a read-only page
whose width was measured from generated text. That made the existing `:about`
path unsuitable for an editable buffer: measuring live text would move the
viewport as lines changed, and the existing `only_window` operation would
destroy the panes a second toggle needed to restore.

`ContentLayout` now also represents a fixed-width viewport, reusing the same
presentation-only indent calculation without moving any buffer offset.
`App::toggle_zen` records the active pane as the sole pane prepared across the
editor area while leaving the underlying panes and `Layout` untouched. The
text region is capped at the typed `editor.zen_width` setting, which defaults
to 100, is validated from 1 through 1000, and is editable through `:config`.
A second `:zen` removes that presentation state and reveals the exact prior
split tree. Narrow terminals use all available text cells.

Zen deliberately follows `editor.soft_wrap` and the ordinary line-number and
gutter settings rather than silently changing unrelated editor preferences.
Pane splitting and pane closing are refused while Zen is active so hidden
layout mutations cannot make the promised restoration ambiguous. This is a
Runyte writing view rather than a claim of Helix compatibility; the NeoVim
feature was treated as product inspiration, not as a behavioral contract.

Tests covering the behavior are
`zen_maximizes_the_active_pane_and_the_second_toggle_restores_the_split_tree`,
`zen_keeps_the_buffer_editable_and_does_not_move_as_text_changes`,
`zen_width_is_configurable_and_narrow_panes_use_every_available_cell`, and
`window_structure_stays_stable_until_zen_is_toggled_off` in
`tests/maximized_panes.rs`, which is where that file was renamed when
`:fullscreen` joined `:zen` as the second maximized view;
`a_fixed_viewport_is_centered_and_capped_without_measuring_text`
in `src/content_alignment.rs`; and
`zen_width_defaults_to_one_hundred_and_is_configurable_and_validated` in
`src/config.rs`.

Known limitation: changing the pane structure requires toggling Zen off first.

## Report

Runyte needed a `:zen` mode aimed primarily at writing prose. The mode was to
center an editable buffer viewport in the manner of `:about`, use a default
width of 100 with a configurable value, and maximize the active pane. Running
`:zen` again was to be the only way to deactivate the mode, making it a
toggle. A similar NeoVim feature was the inspiration. The design was expected
to reuse the centered-content components where possible and provide a
consistent, coherent user experience.
