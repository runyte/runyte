---
title: "File and explorer pane titles did not identify their buffer type"
status: resolved
reported: 2026-08-11
resolved: 2026-08-11
legacy_commit: 031138a
---

## Resolution

Commit `031138a` (`show buffer types in pane titles`) added
`Buffer::pane_title` as the structural naming boundary for pane titles. File
paths are now rendered as `[file] <path>` and directory paths as
`[explorer] <path>`. `App::snapshot_pane` uses that value while the ordinary
`display_name` remains unchanged for buffer pickers, status messages, and
identity comparisons.

Virtual views already encode their kind in bracketed names, so Git status and
Git branches retain `[git status]` and `[git branches]` rather than receiving a
second prefix. This keeps the rule centralized without spreading buffer-kind
matches through the TUI renderer.

Covered by
`buffer::tests::pane_titles_prefix_paths_with_their_structural_buffer_kind` in
`src/buffer.rs` and `pane_titles_show_structural_file_and_explorer_types` in
`tests/key_hints.rs`.

## Report

Some pane titles, such as Git status, already displayed a bracketed buffer
name. Text-file panes displayed only the file path, and explorer panes
displayed only the directory path. The requested consistent form was
`[file] /home/path/to/file` for files and
`[explorer] /home/path/to/dir` for explorers, while retaining the already
appropriate Git status and Git branches names. The implementation was expected
to derive these labels structurally.
