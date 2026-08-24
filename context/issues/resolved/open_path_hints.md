---
title: "Colon path commands treated home-relative paths as working-directory paths and offered no filesystem hints"
status: resolved
reported: 2026-08-15
resolved: 2026-08-15
legacy_commit: 8ccb5a8
---

## Resolution

Commit `8ccb5a8` (`Fix command path hints and home expansion`) corrected
`App::resolve_working_path`, which previously distinguished only absolute paths
from paths relative to the editor working directory. It consequently treated
the `~` in `~/.bashrc` as a literal directory name below that working directory.
The app now captures the user's home directory once, through an injectable
field, and expands a leading `~` before every shared working-path resolution.

The command palette also previously kept projecting the matched command after
its path argument began, because `App::matching_commands` and
`ui::draw_command_palette` had no filesystem-argument projection. The added
`App::matching_path_hints` reads the existing `CommandSpec` argument kind, so
all path-valued colon commands use one bounded hint provider. It preserves
relative, absolute, and `~` spellings; sorts directories before files; retains
a trailing separator for descent; reveals dotfiles after a dot prefix; and
quotes names containing spaces while leaving the prompt cursor inside a
completed directory's quotes. Selecting a directory for `:open` deliberately
uses the existing `App::open_file` directory branch, which retargets the pane's
editable explorer rather than creating a file buffer. This is recorded as a
Runyte command-palette deviation rather than a claim of general Helix command
line compatibility.

Tests covering the behavior are in `src/app.rs`:

- `path_commands_hint_files_and_open_selected_directories_as_explorers`
- `open_expands_home_paths_for_files_and_directory_explorers`
- `path_hint_quotes_spaces_and_keeps_the_cursor_inside_directory_quotes`

Known limitation: one hint refresh examines at most 512 directory entries and
omits names that cannot be represented as UTF-8. Directly typed paths are not
subject to the enumeration bound.

## Report

Running `:open ~/.bashrc` attempted to create or open a `~/.bashrc` path below
the current working directory instead of opening `/home/user/.bashrc`. The
`:open` command needed to support absolute and home-relative paths and provide
filesystem path hints. Directory paths also needed to be offered and to open
the editable explorer at that directory.
