---
title: "Remove the conflicting Vim input grammar"
status: resolved
reported: 2026-08-11
resolved: 2026-08-11
legacy_commit: 856cced
---

## Resolution

Commit 856cced (`remove selectable Vim grammar`) removed Vim from every
production selection surface. `GrammarKind` had continued to expose Vim
through configuration, command parsing, settings, help, and the public active
grammar even though Runyte's selection-first motions now conflict with those
semantics. Configuration and `:grammar` now accept only Runyte (with `helix`
retained as its compatibility alias), settings expose only that choice, and
help has one Runyte-owned set of prose and key rows.

The `GrammarKind::Vim` value is deliberately retained as a non-deserializable
compatibility sentinel so older typed callers receive the explicit `vim
grammar has been removed; use runyte` error. The legacy interpreter and its
unit-test coverage are compiled only for tests; neither `VimGrammar` nor an
active Vim variant exists in production builds.

Coverage lives in
`tests/headless_editor.rs::headless_grammar_report_uses_typed_colon_identity_without_key_events`,
`tests/key_hints.rs::removed_vim_configuration_falls_back_to_runyte_help_and_hints`,
`src/config.rs::tests::editor_grammar_is_typed_and_accepts_the_helix_compatibility_name`,
and the Runyte-only help tests in `src/help.rs`. Run them with `cargo test
--test headless_editor`, `cargo test --test key_hints`, and `cargo test --lib`.

## Report

Normal mode includes familiar Vim and Helix motions, including `g l` and `$`.
Runyte also relies on motions that conflict with Vim, including `v`, `V`, and
`Alt-V`, and its multicursor model behaves differently from Vim's.

Maintaining a separate Vim mode therefore created conflicts and duplicated
help-page maintenance. Runyte should keep only its current input mode.
