---
title: "The theme-list command duplicates the current theme settings UI"
status: resolved
reported: 2026-08-13
resolved: 2026-08-14
legacy_commit: f9cc74b
---

## Resolution

Commit f9cc74b (`Retire the theme-list command`) removed the obsolete
`:theme-list` path. It had maintained a parallel theme projection through
`ColonCommand::ThemeList`, `BufferKind::ThemeList`, a dedicated binding scope
and Enter action, a help topic, and refresh logic in `App`, even though a bare
`:theme` and the `theme` row in `[config]` already shared the newer settings
popup and persistence path.

The command, buffer kind, scoped binding, activation and refresh functions,
help topic, and user-facing documentation were removed together. The generic
read-only UI coverage now uses `[config]`, and the existing `:theme` and config
tests continue to cover theme preview and persistence.

The retired command is covered by
`src/command.rs::tests::settings_and_theme_commands_use_the_new_public_spellings`.
The retained behavior is covered by
`src/app.rs::tests::bare_theme_opens_the_theme_setting_choices`,
`src/app.rs::tests::focused_theme_setting_previews_without_remembering_and_saves_on_enter`,
and `tests/key_hints.rs::a_read_only_buffer_is_marked_on_every_surface`.

## Report

The `:theme-list` command opened a floating theme-selection surface that had
become redundant after the addition of the `[config]` buffer. Theme selection
was already available through `Space o o` on the `theme` setting and through
`Space o t`.

The requested behavior was to retire `:theme-list`, while retaining `:theme`
and the theme setting in `[config]`.
