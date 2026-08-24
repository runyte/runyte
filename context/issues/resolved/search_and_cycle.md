---
title: "Search results do not identify the current match"
status: resolved
reported: 2026-08-11
resolved: 2026-08-11
legacy_commit: 5d69275
---

## Resolution

Commit 5d69275 (`mark the current search match`) made the selection's existing
primary range visible during search cycling. `step_search` had been replacing
the multi-selection with one range, while snapshot construction classified
every selection head as the same caret role. It now rebuilds the complete match
selection with the next or previous match designated as primary, and Select
mode gives that primary caret the normal-cursor colour. Insert mode deliberately
keeps every caret on the configured insert colour.

`Space s c` now invokes the existing `keep-primary-selection` command, with `,`
advertised as its short alias, so the marked match can be retained explicitly.
The binding registry, help text, README, and Helix deviation reference describe
the same behavior.

Coverage lives in
`src/app.rs::tests::search_prompt_repeats_and_wraps_unicode_matches`,
`src/snapshot.rs::tests::a_multiselection_marks_its_primary_caret_separately`,
`src/ui.rs::tests::editor_caret_uses_the_theme_color_for_each_mode`, and
`tests/key_hints.rs::the_search_namespace_and_its_prompts_are_discoverable_on_screen`.
Run them with `cargo test --lib` and `cargo test --test key_hints`.

## Report

Searching with `s`, `S`, or `/` can select multiple matches, with a cursor at
the end of each match. `n` and `N` cycle through those matches, but every cursor
looked the same, so the current position was not visible.

The current selection needed a distinct visual marker while preserving the
multicursor model. A binding such as `Space s c` was also needed to keep only
the current match after cycling.
