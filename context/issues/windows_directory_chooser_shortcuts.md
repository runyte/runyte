# Directory chooser shortcuts are unavailable on Windows

On Linux and macOS, the session manager's Ctrl-o **Open directory…** chooser
lists recent project roots (labelled `<name> · recent root`) and sibling Git
worktrees (labelled `Git worktree`) when its query is empty. These rows open the
persistent session for that directory without typing or browsing to it.

On Windows the chooser accepts drive paths and backslashes, and browsing works,
but neither kind of shortcut row appears. The rows are built in
`rebuild_session_directory_chooser` in `src/app/workspace_workflows.rs`, inside
a `#[cfg(unix)]` block that reads `workspace_rows` and
`session_navigation.worktrees`. `docs/user-guide.md` lists the gap under
"Not available on Windows".

Expected: with an empty query, the Windows chooser offers the same recent-root
and worktree rows as Unix, when the native session catalog and Git provide
them, in their own places after the child directories.

Constraints:

- Windows rows come from the native catalog, whose recent-history names are
  only a cache (`context/plans/completed/WINDOWS_STOPPED_NAMES.md`). A
  shortcut must not use a cached name or path to resolve to a different
  publication.
- Git is optional on Windows. Without it, worktree rows are omitted, and the
  chooser does not report an error.
- Paths that Windows refuses as working directories, such as those whose
  ordinary spelling is 260 UTF-16 units or longer, are refused as they are for
  typed paths.
- When the rows are enabled, remove the entry from the Windows section of the
  user guide.
