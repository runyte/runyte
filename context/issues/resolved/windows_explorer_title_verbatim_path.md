---
title: "Windows explorer title shows a verbatim path prefix"
status: resolved
reported: 2026-09-21
resolved: 2026-09-21
commit: 4477764
---

## Resolution

Commit `4477764` (`Show ordinary Windows paths in pane titles`) changed
`Buffer::pane_title`, which previously formatted the stored path directly and
therefore exposed the `\\?\` prefix returned by Windows path resolution.
`pane_title_path` now renders ordinary drive and UNC spelling for names that do
not require verbatim interpretation. It changes the title only; the buffer
retains its original path for filesystem operations. The same title rule
applies to file and explorer buffers, and the UI vocabulary records it.

Coverage in `src/buffer.rs` is
`pane_titles_show_ordinary_windows_paths_when_the_spelling_is_safe` and
`pane_titles_keep_verbatim_spelling_when_ordinary_names_can_differ`.

Known limitation: paths with reserved or trailing-dot names, or other verbatim
forms without an ordinary drive or UNC spelling, retain the prefix in titles
so that the displayed name does not imply a different path.

## Report

On Windows, opening the explorer could show a pane title like:

```text
[explorer] \\?\C:\Users\...
```

The `\\?\` prefix is Windows' extended path syntax. It may be needed for
filesystem operations, but an ordinary directory should have a recognizable
pane title such as `C:\Users\...`. Reproduce by opening an explorer for a
Windows directory whose stored path is in verbatim form and reading its title.
The stored filesystem identity, including long and UNC paths, must remain
unchanged by title presentation.
