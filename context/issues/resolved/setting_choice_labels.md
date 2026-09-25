---
title: "Setting value lists label every row with choice or effective"
status: resolved
reported: 2026-09-25
resolved: 2026-09-25
commit: b17dd3b
---

## Resolution

Commit `b17dd3b` (`Clarify setting and explorer choice labels`) fixed the
list-row details. `open_setting_values` in `src/app/settings_workflows.rs`
previously labelled the in-use value `effective` and every other unsaved
value `choice`; `choose_explorer_order` in `src/app/file_workflows.rs` used
different words for the same state. Both now label the in-use value
`selected`, a different persisted value `saved`, and other rows with no
detail. Highlighting still marks the row being previewed, independently of
the `selected` label. Empty details neither alter value alignment nor add a
filter match.

`src/app/tests/presentation_and_settings.rs` covers settings and explorer
row states, filtering and preview distinction. `src/ui.rs` covers the width
of an empty detail, and `tests/directory_buffer.rs` covers the explorer
selection flow. Focused tests, formatting and Clippy passed.

## Report

Choosing a setting from the `[config]` settings buffer opens a value list for
finite settings (booleans, grammar, theme, session strip, workspace mode,
explorer sort). Each row carries a dim label beside the value. The labels
come from `open_setting_values` in `src/app/settings_workflows.rs`:

- `effective`: the value the running editor currently uses
- `saved`: the persisted value, when it differs from the one in use
- `choice`: every other value

Example for `editor.smart_newline`:

```text
editor.smart_newline · Enter to save · Esc cancel
> type to filter
  true   choice
▸ false  effective
```

Problems with these labels:

- `choice` adds nothing. Every row in the list can obviously be chosen, so
  the label is noise on most rows.
- `effective` is unclear as a user-facing word for the value in use.

The explorer's listing-order list, `choose_explorer_order` in
`src/app/file_workflows.rs`, shows the same concept with different words:
`in use`, `saved` and `choice`.

## Expected behavior

- Rows that are neither in use nor saved have no label. Remove `choice`.
- Label the value in use `selected` instead of `effective`.
- Keep `saved` for a persisted value that differs from the one in use.
- The explorer listing-order list uses the same words as the settings value
  lists: `selected`, `saved`, and no label otherwise.

Example after the change:

```text
editor.smart_newline · Enter to save · Esc cancel
> type to filter
  true
▸ false  selected
```

Constraints:

- A row with no label must still align and filter like the others. The
  empty detail must not leave stray padding that changes the value column,
  and filtering must not match the removed word.
- Only the row labels change. The `choice` type description shown by the
  setting popup in `src/ui.rs`, the plugin `choice` field type, and overlay
  hints such as `←/→/Space choice` are separate uses of the word and stay as
  they are.
- `selected` names the value in use, not the highlighted row (`▸`). The two
  can differ while the list is open, for example when the highlight moves to
  another value to preview it. Keep them visually distinct, as now.

## Coverage

Add tests in `src/app/tests/presentation_and_settings.rs` that open a
boolean setting's value list and assert the row details: the value in use is
`selected`, a different persisted value is `saved`, and any other row has no
label. Add a matching assertion for the explorer listing-order list. Update
any user-guide or reference text that quotes the old labels.
