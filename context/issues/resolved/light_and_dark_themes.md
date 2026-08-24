---
title: "No themes named dark and light, and no way to see what themes exist"
status: resolved
reported: 2026-07-31
resolved: 2026-07-31
legacy_commit: f320931
---

## Resolution

Fixed by f320931 "Add dark and light themes and a browsable theme list".

Theme machinery already existed — `Config::resolve_theme`, `:theme <name>`,
and three built-in palettes — so nothing here was broken. Two things were
missing.

`dark` and `light` did not exist as names. They are now built-ins in
`Config::default`, deliberately neutral: no palette identity of their own, just
a legible default for each kind of terminal. `base16`, `paper`, and `gruvbox`
are untouched, and the startup default is still `base16`. Changing which theme
Runyte opens with was not asked for, so it was left alone.

`:theme` took a required argument, so the only way to reach a theme was to
already know its name. With no argument it opens the list. That list is a new
`BufferKind::ThemeList` rather than a `Virtual` buffer with a reserved display
name, because Enter has to mean "activate the theme on this line" there and
identity by name would be claimed by any file called `[themes]`. The kind is
read-only through the same `is_read_only` gate virtual buffers use, so `apply`,
`undo`, `redo`, `save`, and `save_as` all refuse it and `Ctrl-s` is stopped
earlier still by the mutating-command gate.

Enter is bound to a new `activate-theme` command under a new
`BindingScope::ThemeList`. A scope rather than a special case in dispatch,
because that is how the explorer's Enter already works and it is what keeps
the binding from leaking into text buffers, where Enter must stay unbound.

Each row carries a fixed two-column marker, `"* "` on the active theme and
`"  "` on the rest, and `Buffer::theme_at` reads the name back by skipping
those two columns rather than by trimming them. Trimming would corrupt a
configured theme whose name begins with a space or an asterisk; theme names
are YAML keys and can hold anything. Activating rebuilds the list in place so
the marker follows, and reopening reuses the single theme buffer instead of
stacking copies. `HelpTopic` gained a `Themes` variant, since the list is now
a view a reader can be standing in when they type `:?`.

Tests: `dark_and_light_are_built_in_and_listed_in_a_stable_order` and
`dark_and_light_put_their_text_on_the_opposite_side_of_their_background` in
`src/config.rs`;
`a_theme_list_marks_the_active_theme_and_names_a_theme_per_row`,
`only_a_theme_list_answers_theme_at`, and
`theme_names_survive_the_marker_column_verbatim` in `src/buffer.rs`;
`theme_names_activate_the_matching_theme`,
`bare_theme_opens_a_read_only_list_whose_enter_activates`, and
`reopening_the_theme_list_reuses_its_one_buffer` in `src/app.rs`; and
`the_theme_list_has_a_topic_of_its_own` in `src/help.rs`.

Known limitation: the list is a projection of the configuration read at
startup, so a theme added to `config.yaml` while Runyte is running does not
appear until it is restarted. There is no preview — Enter activates
immediately, and returning to the previous theme means selecting it again.

## Report

Multiple themes were planned for later; this issue covers a basic
implementation of default light and dark themes.

- `:theme dark` and `:theme light` select them directly.
- `:theme` alone opens a list of available themes, navigable like a normal
  buffer, where Enter activates the theme under the cursor.
- The theme buffer is read-only.
