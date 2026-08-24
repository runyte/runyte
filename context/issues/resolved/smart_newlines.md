---
title: "Disabling smart newline also removes existing indentation"
status: resolved
reported: 2026-08-13
resolved: 2026-08-14
legacy_commit: a322ebd
---

## Resolution

Commit a322ebd (`Preserve base indentation without smart newline`) fixed
`App::edit_newline`. The function returned a bare newline as soon as
`editor.smart_newline` was false, before it derived the insertion row's exact
leading tabs and spaces. Disabling the option therefore disabled both advanced
indentation and the base indentation that should always remain.

The insertion row and its leading prefix are now computed first. With the
option disabled, Enter inserts that prefix after the newline but still bypasses
list hanging alignment and syntax-added indentation. With the option enabled,
the existing list and syntax behavior is unchanged. The setting description,
configuration example, README, and keymap reference now distinguish the
unconditional base indentation from the optional smart behavior.

The behavior is covered by
`src/app.rs::tests::disabled_smart_newline_keeps_leading_indent_without_list_alignment`
and
`src/app.rs::tests::disabled_smart_newline_does_not_add_syntax_indentation`.

## Report

When `editor.smart_newline` was enabled, Enter preserved indentation and added
list-item or syntax-aware indentation. When it was disabled, Enter removed all
indentation instead of disabling only the smart part.

With the option disabled, pressing Enter after `1. First line` was expected to
start `I am here now` without indentation. Pressing Enter after an already
indented continuation such as `   Second line` was expected to preserve those
three spaces before `I am here now`. With the option enabled, pressing Enter
after `1. First line` was expected to keep the existing hanging-indent
behavior and place the next text under the list item's content.
