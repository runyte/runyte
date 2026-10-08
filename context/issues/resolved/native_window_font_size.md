---
title: "Native window font size cannot be adjusted"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: 2766482
---

## Resolution

Commit `2766482` — Fix native window font sizing, clipboard integration, and build warning.

`NativeView` and the cell painter used fixed 15-pixel text and 9×20-pixel
cells; media and pointer calculations repeated those constants. Updating only
the glyph size would therefore have separated visible text from hit targets
and terminal geometry.

`editor.font_size` is now a validated settings-page integer from 8 through 48,
with a default of 15 logical pixels. A window reads its launch configuration
before opening, including when it attaches to a retained host. Saving the
setting or reloading config marks it as startup-bound. `CellMetrics` supplies
font size, cell width and cell height to painting, pointer conversion, media
bounds, IME caret placement and PTY resizing. Temporary Ctrl-plus/Ctrl-equals
and Ctrl-minus adjustments live in `NativeView`, invalidate the glyph cache,
and never write the configuration or retained host state. The reserved window
control registry also supplies their contextual help.

Coverage: `native_font_size_is_bounded_persisted_and_read_on_next_launch` in
`src/settings.rs`; `font_size_reload_preserves_running_value_and_records_next_window_default`
in `src/app/tests/config_reload.rs`;
`native_help_lists_the_window_registry_controls_only_for_window_frontends`
in `src/app/tests/native_media.rs`;
`window_font_controls_share_their_help_and_preserve_other_input` in
`src/keymap/native_window.rs`; and
`resized_font_scales_pointer_and_media_hit_testing_together` in
`src/native_frontend/tests/input.rs`. `tests/native_window.py --window-controls`
and its `--mux` variant passed against the release binary on isolated X11,
checking PTY dimensions, both increase spellings, decrease, unchanged config,
and the saved size after reopening an existing persistent session.

Known limitation: changing the saved font size requires reopening the window.
Window-local font shortcuts are reserved frontend controls, not remappable
editor commands. macOS and Wayland interactions were not exercised locally.

## Report

The native window uses a fixed font size. The starting size should be editable
in the configuration opened with `Space o o`. `Ctrl +` and `Ctrl -` should
adjust the running window's font size temporarily; after restart it should
read the configured size again. Pointer targeting, pane geometry and integrated
terminal dimensions must stay consistent with the displayed text.
