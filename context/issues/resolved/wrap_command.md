---
title: "Wrapping lacked word-aware hard and soft commands"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: 7829ab9
---

## Resolution

Commit 7829ab9 (`Add selection wrapping commands`) fixed the wrapping surface and
the geometry behind it. `wrap::segments` had split visual rows only at the next
character whose display width crossed the pane boundary, so ordinary words
could be divided even when preceding whitespace offered a valid break. It now
prefers whitespace boundaries and falls back to character boundaries only for
a word wider than the available pane.

The same commit added the registry-owned `Wrapping` namespace. `Space p h`
opens a width prompt and replaces every selected span through one transaction;
an empty prompt reads `editor.hard_wrap_width`, whose validated default is 80,
while a typed positive width applies only to that edit. `Space p s` toggles the
runtime `editor.soft_wrap` value. Soft wrapping continues to derive its width
from each live pane, so resizing a pane changes wrapping independently of the
hard-wrap setting. The hard-wrap setting is also part of the typed settings
registry and lossless YAML editor.

Coverage is in `wrapping_namespace_hard_wraps_with_default_or_typed_width_and_toggles_soft_wrap`
in `src/app.rs`, `segments_wrap_at_words_and_split_only_overlong_words` and
`hard_wrap_uses_word_boundaries_and_preserves_existing_newlines` in
`src/wrap.rs`, `hard_wrap_width_is_configurable_and_validated` in
`src/config.rs`, `validation_is_typed_and_bounded` in `src/settings.rs`, and
`core_minor_modes_expose_registry_continuations` in `tests/keymap.rs`. Run
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
`cargo test`.

Known limitation: hard wrapping preserves existing newline boundaries; it does
not join already wrapped physical lines into a single reflowed paragraph.

## Report

A command namespace was requested for wrapping:

- `Space p h <number>` — hard wrap
- `Space p s` — toggle soft wrapping, changing the setting from the config

The default hard-wrap width is 80 and is configurable.

- `Space p h <Enter>` on a selection hard-wraps it to the default width.
- `Space p h 100 <Enter>` hard-wraps it to 100 characters.
- Neither hard nor soft wrapping breaks words. The one exception is a line
  consisting of a single word longer than the limit, which may be broken.
- The soft-wrap toggle stays in the config, as does the hard-wrap limit.
- Soft wrapping always follows the pane width dynamically rather than the
  hard-wrap limit.
