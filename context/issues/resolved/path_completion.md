---
title: "Typing filesystem paths does not offer completion candidates"
status: resolved
reported: 2026-08-13
resolved: 2026-08-14
legacy_commit: 06212b1
---

## Resolution

Commit 06212b1 (`Add automatic filesystem path completion`) fixed
`App::after_insert`, whose automatic completion path was limited to LSP
requests after `.` and `:`. Typing `/` therefore had no filesystem completion
source and could not open a popup without a language server.

The completion state now records whether its source is the language server or
the filesystem. `App::path_completion` extracts the current line-local path
token, resolves absolute paths directly and relative paths against both the
active file's parent and the stable project root, and offers files and
trailing-slash directory candidates in the existing filterable popup.
Accepting a directory immediately offers its children. Late LSP responses do
not replace an active path popup, filename punctuation remains filterable, and
synchronous enumeration is capped independently for each candidate root.

The behavior is covered by
`src/app.rs::tests::slash_completes_paths_from_the_file_directory_and_project_directory`,
`src/app.rs::tests::path_completion_filters_filename_punctuation_and_continues_into_directories`,
`src/app.rs::tests::late_language_responses_do_not_replace_an_active_path_completion`,
`src/app.rs::tests::path_completion_bounds_directory_enumeration`, and
`src/app.rs::tests::a_full_file_directory_does_not_hide_project_root_path_candidates`.

Known limitation: one completion trigger examines at most 512 directory
entries per distinct root, so entries beyond that filesystem enumeration
window are not offered.

## Report

Runyte did not offer file-path completion comparable to the helpers in NeoVim
or Helix. The preferred behavior was Helix-like automatic completion: after a
user typed a valid absolute or relative directory path and `/`, a popup should
show completion candidates without requiring an explicit trigger.

Relative paths needed to resolve from both the active file's directory and the
project directory. In the reported example, the project directory was
`/home/user/project`, the active file was
`/home/user/project/files/a.txt`, another file was
`/home/user/project/files/dir/b.txt`, and another directory was
`/home/user/some_dir`. Completion was expected for absolute paths and for the
spellings `dir/`, `files/`, `./files/`, `../files/`, and `../some_dir/`.
