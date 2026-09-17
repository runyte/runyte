# A pasted value containing a control character is discarded whole by plugin prompts

Pasting a value into a plugin input surface inserts nothing and reports
`Complete the selected field within its declared limits` when the pasted text
contains any control character. One source is the trailing newline
that a path or other value keeps when it is copied from terminal output, a file
listing, or a file manager.

The message names limits, but no limit was reached. The reported length limits
are 4,096 bytes and, for the field in the reproduction below, 4,096 characters,
while the path was 258 characters (259 with the trailing LF).

## Observed behavior

A bracketed paste arrives as one `InputEvent::Text`. The insertion guard in
`src/app/plugin_interaction.rs:364` accepts the text only if all three
conditions hold:

```rust
if value.len().saturating_add(text.len())
    <= crate::plugin::interaction::MAX_VALUE_BYTES
    && value.chars().count() + text.chars().count() <= field.maximum_length
    && !text.chars().any(char::is_control)
{
    for c in text.chars() {
        prompt_insert(value, surface.cursor, c);
        surface.cursor += 1;
    }
    surface.error = false;
} else {
    surface.error = true;
}
```

The paste is atomic: one control character anywhere in it rejects the whole
insertion, so the field is left exactly as it was. `surface.error` is then
rendered at `src/app/presentation.rs:2113` as the single message
`Complete the selected field within its declared limits`, which describes only
the first two conditions.

To the person at the prompt, the field stays empty and the editor reports a
limit. Nothing identifies the control character as the cause, and nothing
distinguishes this from a value that was genuinely too long.

`surface.error` has a second producer. On Enter, `src/app/plugin_interaction.rs`
selects the first field for which `Field::accepts` is false and sets the same
flag, so submit-time rejections — including a required field left empty — are
reported with the same sentence. `Field::accepts`
(`src/plugin/interaction.rs:74`) also rejects any value containing a control
character.

## Expected behavior

The report did not decide the expected behavior. Two candidates were proposed,
with different implications for the values a plugin receives:

- Strip control characters from pasted text, or truncate the paste at the first
  one, and insert the rest, on the basis that a single-line field cannot
  represent a newline.
- Keep the rejection but report the actual cause, distinguishing a control
  character from a length violation.

Either way, a rejection should not report a limit that was not reached.

## Inconsistency with native prompts

Plugin input surfaces are stricter than the editor's own prompts, which apply
no control-character check to pasted text:

- `src/app/input.rs:1632` inserts pasted text into the command line verbatim.
- `insert_confirmation_text` (`src/app/input.rs:5708`) does the same for the
  Git branch-switch, branch-deletion, and worktree-removal confirmations.
- `src/app/input.rs:1602` pushes each pasted character into a filterable list's
  filter.

The same paste that a plugin prompt discards entirely is therefore accepted by
the editor's own single-line surfaces.

## Reproduction

Observed with a plugin that opens a two-field form, the second field being a
filesystem path declared with `maximum_length` 4096. Driving a real editor over
a pseudoterminal, with a database at a 258-character path:

- Typing the path character by character, then Enter: the field accepts it and
  the plugin proceeds. No error.
- Pasting the path as `ESC [ 200 ~ <path> ESC [ 201 ~`: the field accepts it.
  No error.
- Pasting the same path with a trailing newline inside the brackets,
  `ESC [ 200 ~ <path> LF ESC [ 201 ~`: the error appears and no part of the
  path is inserted.

The third case is the report. The first two establish that neither the length
of the path nor pasting itself is the trigger.

## Constraints

- `MAX_VALUE_BYTES` is 4,096 (`src/plugin/interaction.rs:10`), and a field's
  `maximum_length` may not exceed it. Text input as a whole is capped at
  `MAX_TEXT_INPUT_BYTES`, 1 MiB. None of these bounds is involved in the
  reported failure.
- Whatever a field ends up holding must continue to satisfy `Field::accepts`
  before it is submitted to a plugin, so relaxing the insertion guard without
  relaxing `accepts` would let a field hold a value it can never submit.
- Secret fields take the same path, so the behavior applies to pasted
  passwords as well as pasted paths.
