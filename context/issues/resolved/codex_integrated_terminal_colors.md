---
title: "Codex theme colors are wrong in Runyte's integrated terminal"
status: resolved
reported: 2026-08-20
resolved: 2026-08-21
legacy_commit: d336002
---

## Resolution

Commit `d336002` (`Report default terminal colors to child programs`) corrected
`Emulator::osc`, which discarded the read-only OSC 10 and OSC 11 requests a
child uses to discover its terminal's default foreground and background.
Runyte still rendered default cells with the active editor theme, so Codex
could choose dark fallback colours while its cells were displayed on Runyte's
light background.

`TerminalSessions` now owns the effective default RGB pair and supplies it to
new and existing emulators. `App` synchronizes that pair at startup and across
theme preview, cancellation, and saving. An emulator answers only exact
read-only `OSC 10;?` and `OSC 11;?` requests, using the standard 16-bit
`rgb:rrrr/gggg/bbbb` form through the existing bounded child-reply path. A
theme colour set to `reset` receives no answer because Runyte cannot know the
outer terminal's resolved RGB value. Colour setters, palette queries, and OSC
52 remain deliberately unsupported.

When Runyte's branded `default-dark` theme later became the application
default, the real-PTY OSC 11 fixture was updated to expect that theme's
`#16181d` background. The emulator behavior was already correct; the fixture
had retained the previous default theme's `#fbfbfa` value.

The behavior is covered by
`default_colour_queries_answer_with_the_current_theme_colours` and
`default_colour_queries_do_not_expand_into_palette_or_clipboard_access` in
`src/terminal/emulator.rs`,
`changed_default_colours_reach_existing_session_emulators` in
`src/terminal/mod.rs`, `theme_names_activate_the_matching_theme` and
`focused_theme_setting_previews_without_remembering_and_saves_on_enter` in
`src/app.rs`, and `a_child_can_discover_the_effective_default_background` in
`tests/terminal.rs`.

Known limitation: when either base theme colour is `reset`, the corresponding
query remains unanswered because Runyte does not know the outer terminal's
effective colour.

## Report

When Codex runs in Runyte's integrated terminal, Codex's theme colors are
wrong. The same Codex session looks correct when run normally in Alacritty or
tmux.
