# Tab always inserts spaces, with no setting for tab indentation

In Insert mode, `Tab` runs `insert-tab`. `insert_indentation` in
`src/app/editing.rs` inserts spaces up to the next multiple of
`editor.tab_width`, whatever the file type, grammar or existing indentation.
`Shift-Tab` runs `insert-literal-tab`, which inserts a tab character. There
is no setting to change which of the two `Tab` inserts.

The other indentation commands behave as follows:

- `>` (`indent`) always adds `tab_width` spaces to each selected line.
- `<` (`unindent`) removes one leading tab, or else up to `tab_width` leading
  spaces.
- Replace-mode `Tab` overwrites with `tab_width` spaces, and Replace-mode
  `Shift-Tab` with a tab.
- `Enter` copies the current line's leading whitespace exactly. With
  `editor.smart_newline` on, it may add one more level. That level is
  `tab_width` spaces, or a tab when the grammar's indent query reports
  `tab_levels`.

In a tab-indented file, `Tab`, `>` and smart newline therefore mix spaces
into tab indentation. The only way to type a tab is `Shift-Tab`.

## Expected behavior

Add a setting `editor.indent` with two values, `spaces` and `tabs`. The
default is `spaces`, so existing behavior is unchanged when the setting is
absent:

```yaml
editor:
  tab_width: 4
  indent: spaces # or tabs
```

| Key | `indent: spaces` (default) | `indent: tabs` |
| --- | --- | --- |
| Insert `Tab` | spaces to the next `tab_width` stop | one tab character |
| Insert `Shift-Tab` | one tab character | spaces to the next `tab_width` stop |
| `>` | `tab_width` spaces per line | one tab per line |
| smart newline extra level | `tab_width` spaces | one tab |

- `Tab` always inserts the configured style, and `Shift-Tab` always inserts
  the other one.
- `>` and the smart-newline extra level follow the setting. A grammar indent
  query that asks for a tab (`tab_levels > 0`) keeps producing a tab in both
  modes.
- `<` keeps its current behavior, which already removes either style.
- Replace-mode `Tab` and `Shift-Tab` follow the same mapping as Insert mode.
- `tab_width` keeps both of its roles: the number of spaces in one space
  indent and the display width of a tab. There is no separate width setting.
- The setting is available wherever `tab_width` and `smart_newline` are:
  the config file, the `[config]` settings buffer (`src/settings.rs`),
  validation in `src/config.rs`, and config reload. An unknown value is a
  config error like an out-of-range `tab_width`.

## Command names and help

The current names and descriptions describe a fixed character:
`insert-tab` / "Insert indentation" and `insert-literal-tab` / "Insert a tab
character". Under `indent: tabs` the second would be false. Rename the
commands by role, for example:

- `insert-indent`: insert one indent in the configured style
- `insert-other-indent`: insert one indent in the other style

Help, key hints and the user guide must describe what each key actually
inserts under the current setting, while still reading from the keymap
registry. Examples: "Insert a tab (indent: tabs)", "Insert spaces
(indent: tabs)". Whether the old command names remain as accepted aliases in
`keys` configuration is undecided. Choose while fixing this, and record the
choice.

## Out of scope

- Detecting the indentation style from the file's existing contents.
- Per-language defaults (for example, tabs for Make and Go).
- Reading `.editorconfig`.

A later change can add any of these on top of `editor.indent` as the
fallback, without changing what the setting means.

## Documentation and coverage

- `docs/user-guide.md`: the Insert and Replace modes key table (`Tab` /
  `Shift-Tab`), the smart-newline paragraph, and the example config block.
- `context/reference/helix-keymap-v1.md`: the Insert `Tab` and `Shift-Tab`
  rows. Helix chooses the indent unit per language, and Runyte uses one
  global setting. Record that as a deviation.
- `context/issues/help_lists_every_mode.md` names `insert-tab` and
  `insert-literal-tab`. Update it if it is still open when the commands are
  renamed.
- Tests in `src/app/tests/editing_and_buffers.rs` cover, for both settings,
  Insert `Tab` and `Shift-Tab` with multiple carets at different columns,
  Replace-mode `Tab` and `Shift-Tab`, `>` on a multi-line selection, and a
  smart newline that adds a level. A config test covers the default,
  parsing both values, and rejecting an unknown value.
