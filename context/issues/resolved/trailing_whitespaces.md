---
title: "Text-file saves do not remove trailing whitespace"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: b42d18a
---

## Resolution

Commit b42d18a (`Trim trailing whitespace on save`) added the typed
`editor.trim_trailing_whitespace` configuration setting and enabled it by
default. `App::trim_trailing_whitespace` removes spaces and tabs at the end of
each text line through one normal transaction before file and save-as writes,
so the buffer matches disk and undo can restore the removed characters.
CRLF line endings are preserved. Directory plans keep their existing save
behavior. Setting the option to `false` preserves trailing whitespace.

Coverage is in
`saving_trims_trailing_spaces_and_tabs_by_default_and_can_be_disabled` in
`src/app.rs`,
`trailing_whitespace_trimming_is_default_on_and_configurable` in
`src/config.rs`, and
`registry_has_stable_unique_keys_ids_and_typed_configured_values` in
`src/settings.rs`.

## Report

A configuration option was requested to strip trailing whitespace from a text
file on save, enabled by default.
