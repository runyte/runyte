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

The config document now opts out of the editor-wide visual soft-wrap pass. Its
columns are already physically wrapped to the fixed 100-cell layout; wrapping
those padded rows again at the narrower post-gutter pane width produced an
empty `↪` continuation after nearly every row. Normal horizontal scrolling
remains available when the complete layout is wider than the pane. Motion,
viewport alignment, scrolling, jump labels, and horizontal mouse scrolling
now use that same per-pane soft-wrap decision; previously only rendering did,
so `j` and `k` still stopped on invisible visual segments when soft wrap was
enabled globally.

Coverage is provided by
`settings::tests::config_page_is_one_hundred_cells_wide_and_wrapped_rows_keep_identity`
in `src/settings.rs`,
`app::tests::config_commands_and_binding_open_the_registry_backed_buffer`,
`app::tests::config_vertical_motion_ignores_the_global_soft_wrap_setting`,
`app::tests::enter_on_a_wrapped_config_continuation_opens_that_settings_choices`,
`app::tests::hard_wrap_width_setting_uses_a_typed_prompt_and_persists_on_enter`,
and `app::tests::git_refresh_interval_uses_a_typed_seconds_prompt_and_accepts_zero`
in `src/app.rs`,
`snapshot::tests::typed_setting_prompt_owns_a_popup_and_not_the_message_line`
in `src/snapshot.rs`, and
`ui::tests::numeric_setting_input_renders_as_a_bounded_popup` in `src/ui.rs`.
Popup consistency is covered by
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
