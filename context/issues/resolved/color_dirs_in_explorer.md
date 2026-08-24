---
title: "Explorer directories were not visually distinguishable from files"
status: resolved
reported: 2026-08-11
resolved: 2026-08-11
legacy_commit: d67e86b
---

## Resolution

Commit `d67e86b` (`color explorer directories by theme`) added an optional
`directory` color to theme definitions and the resolved theme. Every built-in
palette supplies its own blue: darker blues for light themes and lighter blues
for dark themes. Existing custom themes remain valid and fall back to their
`accent`; ordinary explorer files retain the standard foreground color.

`DirectoryBuffer::entry_kind_at_line` derives kind from the hidden row origin
for existing and transferred entries, so editing a directory label does not
make its color flicker or change. New rows derive kind from their visible `/`
marker. `App::snapshot` carries that semantic fact in each text run, and the
TUI only maps it to the resolved theme color, preserving the frontend and
directory-model boundaries.

Covered by
`directory_row_kinds_survive_edits_and_transfers_while_new_rows_use_the_marker`
and `a_directory_renders_as_editable_text_with_directory_markers` in
`tests/directory_buffer.rs`,
`snapshot::tests::explorer_rows_carry_directory_semantics_into_the_snapshot`
in `src/snapshot.rs`, `ui::tests::directory_runs_use_the_theme_directory_color`
in `src/ui.rs`, and
`config::tests::built_in_themes_use_palette_specific_blue_directory_colors`
in `src/config.rs`.

## Report

Directories and files used the same foreground color in the explorer.
Directories were expected to have a theme-configurable color while files kept
the standard foreground. Light themes were expected to use blue for
directories, and dark themes a potentially lighter blue.
