---
title: "File picking and content search do not share a complete search namespace"
status: resolved
reported: 2026-08-12
resolved: 2026-08-12
legacy_commit: 0114dd5
---

## Resolution

Commit `0114dd5` (`Add native fuzzy content picker`) resolves the issue. The
keymap registry previously exposed the project and active-directory file
pickers only through `Space f` and `Space F`, while `Space s f` was the
canonical binding for the next-character search. The file picker itself could
rank only paths, and its scanner emitted only file entries, so the existing
literal and regular-expression workspace search did not provide an
interactive fuzzy content picker.

The shared file-picker boundary now distinguishes path and content searches.
Its ignore-aware background scanner streams non-empty UTF-8 lines as bounded
content candidates, and the same smart-case fuzzy scorer ranks their line text.
The list identifies each result as `path:line`, while the selected file's
content stays in the preview. That preview shows up to four lines on either
side of the match, uses real line numbers, marks the target row, and highlights
every fuzzy-match character. Content results retain their file, row, and
indentation-aware column, so `Enter`, `Ctrl-s`, and `Ctrl-v` open the selected
target at the match. Open buffers replace disk results for their paths, which
keeps unsaved text authoritative. The project and active-file-directory roots
use the same root selection rules as their corresponding file pickers.

The first implementation exposed the matching line text in both the result
list and preview because `FileEntry::label` appended content to every content
result. The follow-up keeps content as the hidden fuzzy-ranking candidate and
limits the left column to the path and line number, leaving content presentation
to the right-hand preview. A later preview correction uses the selected result's
stored row and fuzzy positions so that pane shows the match and its context
instead of beginning at the head of the file.

The registry now binds `Space s f` and `Space s F` to the two file pickers,
with `Space f` and `Space F` retained as aliases. `Space s g` and `Space s G`
open project and active-directory fuzzy content search. The direct `f` binding
alone remains next-character search. Because `:help` and prefix hints are
generated from the keymap registry, all four canonical bindings and both file
picker aliases appear there. The same commands are also available as
`:fuzzy-grep` and `:fuzzy-grep-directory`.

Coverage is provided by
`file_picker::tests::fuzzy_grep_ranks_line_contents_and_keeps_a_jump_target`,
`file_picker::tests::content_scan_uses_file_picker_ignore_and_text_boundaries`,
`file_picker::tests::background_content_scanner_streams_lines_and_finishes`,
and `file_picker::tests::content_preview_centers_context_and_preserves_match_emphasis`
in `src/file_picker.rs`;
`app::tests::fuzzy_grep_searches_contents_at_both_roots_and_enter_jumps_to_the_match`
in `src/app.rs`;
`ui::tests::fuzzy_grep_picker_keeps_paths_in_the_list_and_content_in_the_preview` in
`src/ui.rs`; `file_and_content_pickers_are_global_in_every_buffer_scope` and
`find_next_character_keeps_only_its_direct_binding` in `tests/keymap.rs`; and
`the_search_namespace_and_its_prompts_are_discoverable_on_screen` in
`tests/key_hints.rs`.

Known limitation: content search treats each non-empty line as a separate
candidate, skips non-UTF-8 files and files larger than 4 MiB, and caps a scan
at 10,000 candidates. It does not fuzzy-match across line boundaries.

## Report

The file picker should remain available at `Space f` and `Space F`, while file
picking and fuzzy content search should also form a namespace under `Space s`:

- `Space s f` (alias `Space f`) opens the file picker for the project working
  directory.
- `Space s F` (alias `Space F`) opens the file picker for the current file's
  directory.
- `Space s g` opens fuzzy content search for the project working directory.
- `Space s G` opens fuzzy content search for the current file's directory.

`Space s f` should no longer alias the next-character search. The direct `f`
binding remains the next-character search. All of these bindings should be
described correctly in `:help`.
