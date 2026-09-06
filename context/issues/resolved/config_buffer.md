---
title: "The settings picker is inconsistent with normal buffer navigation and clips descriptions"
status: resolved
reported: 2026-08-13
resolved: 2026-08-13
legacy_commit: b969a91
---

## Resolution

Commit `b969a91` (`replace settings picker with config buffer`) replaced
`App::open_settings_menu`, which projected the registry into the shared fuzzy
picker and therefore limited navigation to picker controls, with a reusable
read-only `BufferKind::Settings` projection named `[config]`. The old picker
also concatenated title, description, effective value, and saved value into a
single unwrapped detail field, so the renderer clipped the description at the
popup boundary. `settings::render_settings_page` now lays out stable setting,
description, and saved-value columns in exactly 100 display cells and wraps
each column without losing the `SettingId` attached to continuation rows.

The same commit added a Settings keymap scope whose only view-specific binding
is Enter. Normal and Select motions, search, splits, help, and other global
buffer commands continue to come from the shared keymap. Enter resolves the
current physical row through its stored setting identity, so it works on both
the first row and any wrapped continuation. Finite grammar, boolean, and theme
values retain a list popup and immediate preview behavior. Integer and future
unbounded text settings use a registry-driven popup input; integer titles show
their minimum and maximum, validation rejects out-of-range input, and errors
stay inside the popup instead of moving input below the status line. The
private local protocol was advanced to version 6 because typed setting prompts
now carry their registry key.

A follow-up gave finite-choice and typed setting popups the same compact layout
policy. Both are centered at 60 columns by 9 rows on a terminal large enough
to hold them and clamp to the available editor area on smaller terminals. The
layout intent is also carried in frontend-neutral overlay metadata, so attached
and standalone TUI rendering agree instead of falling back to the general
result-picker percentages.

The physically padded page originally opted out of ordinary visual soft
wrapping because a second wrap pass produced empty continuation rows. Motion
and rendering shared that exception so navigation could not stop on invisible
segments.

A later presentation refinement removed fixed-width column wrapping. The page
now orders its columns as setting, saved value, and description, sizes the first
two from their content, and leaves each setting on one logical line. The theme's
`function` and `constant` scopes distinguish setting names and values;
descriptions remain normal text. `App::generated_highlights` carries those
character-offset spans through the existing frontend snapshots and refreshes
them with the page after a value changes. Ordinary pane-width soft wrapping,
controlled by `Space p s`, replaces the settings-specific opt-out. Different
panes can wrap the same document independently, and Enter on a visual
continuation still resolves its logical row's setting identity.

Current presentation coverage is provided by
`settings_keep_complete_keys_values_and_descriptions_on_single_lines`,
`long_unicode_values_align_by_cells_and_colour_by_character_offsets`, and
`empty_values_and_empty_registry_leave_readable_headers_without_empty_spans`
in `tests/settings_page.rs`. Interaction and snapshot coverage lives in
`src/app/tests/presentation_and_settings.rs`:
`config_commands_and_binding_open_the_registry_backed_buffer`,
`config_navigation_follows_soft_wrap_without_rewriting_shared_text`,
`enter_on_a_wrapped_config_continuation_opens_that_settings_choices`, and
`config_column_colours_reach_snapshots_and_refresh_after_value_width_changes`.
The typed editing tests in that same file include
`hard_wrap_width_setting_uses_a_typed_prompt_and_persists_on_enter` and
`git_refresh_interval_uses_a_typed_seconds_prompt_and_accepts_zero`.
Popup consistency remains covered by
`ui::tests::setting_popups_share_one_compact_fixed_size` in `src/ui.rs`.

## Report

The config window did not feel consistent with the rest of Runyte.
Descriptions extended beyond the window, normal Runyte motions could not be
used to move around, and some settings such as `editor.hard_wrap_width` were
entered in the line below the status line.

Configuration was expected to appear as a read-only `[config]` buffer with
three columns:

- setting name
- description
- value

Setting names and descriptions were expected to wrap so the total width of
the config page was 100 characters. The page was expected to remain left
aligned like `:help`, independently of the separate
`auto_centered_virtual_content.md` task.

The buffer was expected to support normal motions, keybindings, and search.
Its only setting-specific binding was to be Enter. Enter on any row belonging
to a setting, including a wrapped row, was expected to open a popup for its new
value. Settings with a finite set of options were expected to use a list;
numeric settings were expected to accept typed input while showing and
enforcing minimum and maximum values; unbounded string settings were expected
to accept typed input in the popup.
