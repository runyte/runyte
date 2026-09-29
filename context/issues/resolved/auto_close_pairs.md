---
title: "Typed brackets and quotes have no optional matching closer"
status: resolved
reported: 2026-09-29
resolved: 2026-09-29
commit: 2248071
---

## Resolution

2248071 (`Add optional Insert-mode bracket and quote auto-closing`) adds the
live `editor.auto_close` boolean, defaulting to false, to the typed settings
registry. `handle_editor_input` distinguishes a typed character from pasted
text before grammar translation; `insert_typed_character` chooses pairing,
literal insertion or closer skipping independently at each caret from pre-edit
text. One transaction maps every resulting caret. `edit_backspace` removes
adjacent empty pairs when enabled. Replace mode and text paste remain literal.
Quotes require word boundaries and use a bounded backslash-parity check.

Regression coverage: `auto_close_is_opt_in_and_pairs_typed_characters_only`,
`auto_close_respects_quote_boundaries_escapes_and_replace_mode`, and
`auto_close_maps_mixed_multi_caret_edits_and_nesting` in `src/app/tests/editing.rs`;
`auto_close_setting_previews_rolls_back_persists_and_reloads` in
`src/app/tests/config_reload.rs`. The exhaustive registry checks in
`tests/settings_registry.rs` and lossless writes in `tests/settings_persistence.rs`
also pass. `registry_has_stable_unique_keys_ids_and_typed_configured_values`
in `src/settings.rs` checks that the descriptor and identity inventories agree
in ordering as well as membership, including the added auto-close row.

Known limitation: pairing is a local text heuristic, not a syntax-aware string
or comment parser. Closer skipping and empty-pair deletion also apply to pairs
already present in a document; they do not track insertion provenance.

## Report

[Issue #9](https://github.com/runyte/runyte/issues/9) requests automatic closing
of brackets `() [] {}` and quotes `'' ""`. Typing an opener currently inserts
only that character. When enabled, typing an opener should insert its matching
closer and leave the caret between them.

The setting must be available in `Space o o` and disabled by default. The
original report leaves context heuristics, closer skipping and Backspace
behavior unspecified. Literal paste and terminal input must retain their text.
