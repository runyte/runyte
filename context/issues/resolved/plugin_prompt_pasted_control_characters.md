---
title: "Plugin prompts misreport rejected control characters as length violations"
status: resolved
reported: 2026-09-17
resolved: 2026-09-17
commit: 3fc8ff8
---

## Resolution

Commit `3fc8ff8` (`Report specific plugin input validation failures`)
replaces the boolean error flag shared by insertion and
submission with a structured reason and the operation that failed.
`App::handle_plugin_input` had correctly rejected a whole insertion containing a
control character, but `App::overlay_snapshots` could only render the same
length-oriented sentence for every rejection. `Field::validate_value` now reports
control characters, byte limits, character limits, required values, minimum
lengths, invalid choices and incorrect value types; `Field::accepts` delegates to
it so the accepted plugin values remain unchanged. Insertion and submission share
control and upper-bound validation, while lower bounds remain submission-only so
incomplete values can be edited.

Native feedback reports the specific failure without including any rejected
text. The error state retains only a reason and numeric bound, including for
secret fields. A rejected insertion preserves the complete prior value and cursor.
Successful edits, including Backspace/Delete, clear stale feedback, as does field
navigation. Enter clears the prior insertion error and validates again, so a
valid retained value can proceed to asynchronous validation without the earlier
error hiding its pending or returned feedback.

The selected policy preserves atomic rejection, including trailing line endings.
No stripping or truncation was added: either can silently change a path or
password. This resolves the misleading feedback candidate from the report;
automatic normalization remains outside this change.

A real PTY reproduction with a two-field plugin and a database at a
258-character ASCII path confirmed that typed input and clean bracketed paste
submit the exact path and open the database. LF, CRLF, embedded tab, embedded LF,
U+0085 and a dummy secret ending in LF reject the entire insertion with
`Control characters are not allowed; nothing was inserted`. A 4,098-byte value
of 2,049 `é` scalars instead reports the 4,096-byte bound. The plugin's submission
callback verified that rejected pastes preserve prior text. The PTY harness and
database were temporary test artifacts, not repository dependencies.

Regression coverage:

- `control_pastes_preserve_value_cursor_and_secret_masking`,
  `byte_and_character_limits_report_separately_and_accept_exact_bounds`,
  `required_and_minimum_lengths_apply_on_submit_and_select_the_invalid_field`,
  `deletion_clears_rejection_and_enter_revalidates_the_remaining_value`,
  `moving_between_fields_clears_feedback_but_moving_the_cursor_keeps_it`, and
  `rejected_paste_does_not_hide_submission_validation_feedback` in
  `src/app/tests/plugin_validation.rs` cover native editing, feedback,
  submission and asynchronous validation.
- `value_validation_preserves_acceptance_and_distinguishes_failures` in
  `src/plugin/tests/interaction_validation.rs` covers the shared validator,
  Unicode scalar versus UTF-8 byte bounds, optional values, choices and types.
- `form_masks_secrets_in_private_frames_and_returns_unicode_only_to_owner` in
  `src/workspace/host/tests/plugin_interaction.rs` covers rejection feedback in
  bundled-client frames, secret masking, and exact submission to the owner.

Validation passed: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` (3,643 passed,
34 ignored). The canonical workspace coverage run with
`--summary-only --fail-under-lines 89` measured 91.79% line coverage on
`x86_64-unknown-linux-gnu`. Process/socket tests ran outside the sandbox;
coverage used an empty temporary `XDG_CONFIG_HOME` because the existing
diagnostic-log host fixture otherwise inherited user configuration and
failed its shutdown assertion. Native macOS coverage remains a CI check.

The native command line previously accepted a newline-containing paste while
displaying only its first line, so it could execute a filename suffix the user
could not see. The native input follow-up is recorded separately in
`native_prompt_hidden_control_characters.md`; it extends atomic rejection to
native single-line surfaces. Subprocess fixtures inheriting user configuration
are tracked separately in `subprocess_tests_inherit_user_configuration.md`.

## Report

Pasting a value into a plugin input surface inserts nothing and reports
`Complete the selected field within its declared limits` when the pasted text
contains any control character. One source is the trailing newline
that a path or other value keeps when it is copied from terminal output, a file
listing, or a file manager.

The message names limits, but no limit was reached. The reported length limits
are 4,096 bytes and, for the field in the reproduction below, 4,096 characters,
while the path was 258 characters (259 with the trailing LF).

### Observed behavior

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

### Expected behavior

The report did not decide the expected behavior. Two candidates were proposed,
with different implications for the values a plugin receives:

- Strip control characters from pasted text, or truncate the paste at the first
  one, and insert the rest, on the basis that a single-line field cannot
  represent a newline.
- Keep the rejection but report the actual cause, distinguishing a control
  character from a length violation.

Either way, a rejection should not report a limit that was not reached.

### Inconsistency with native prompts

Plugin input surfaces are stricter than the editor's own prompts, which apply
no control-character check to pasted text:

- `src/app/input.rs:1632` inserts pasted text into the command line verbatim.
- `insert_confirmation_text` (`src/app/input.rs:5708`) does the same for the
  Git branch-switch, branch-deletion, and worktree-removal confirmations.
- `src/app/input.rs:1602` pushes each pasted character into a filterable list's
  filter.

The same paste that a plugin prompt discards entirely is therefore accepted by
the editor's own single-line surfaces.

### Reproduction

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

### Constraints

- `MAX_VALUE_BYTES` is 4,096 (`src/plugin/interaction.rs:10`), and a field's
  `maximum_length` may not exceed it. Text input as a whole is capped at
  `MAX_TEXT_INPUT_BYTES`, 1 MiB. None of these bounds is involved in the
  reported failure.
- Whatever a field ends up holding must continue to satisfy `Field::accepts`
  before it is submitted to a plugin, so relaxing the insertion guard without
  relaxing `accepts` would let a field hold a value it can never submit.
- Secret fields take the same path, so the behavior applies to pasted
  passwords as well as pasted paths.
