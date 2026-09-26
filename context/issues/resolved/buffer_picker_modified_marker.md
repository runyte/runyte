---
title: "The buffer picker omits the [+] modified marker"
status: resolved
reported: 2026-09-26
resolved: 2026-09-26
commit: fc2409f
---

## Resolution

Commit `fc2409f` (`Mark modified buffers with [+] in the buffer picker`)
resolves the issue. `buffer_picker_columns` in `src/app.rs` built a row label
from the buffer's name and appended `[STALE]` and `[RO]`, but never consulted
`buffer.dirty`, so a buffer with unsaved changes was listed like a clean one.

The function now appends `[+]` when `buffer.dirty` is set, ahead of `[STALE]`
and `[RO]`. That is the order pane titles and the Navigator use, and the same
flag they read, so `[+] [STALE]` reads as a conflict in all three places.
`buffer_picker_columns` also labels the Finder's buffer rows, which gain the
marker by the same change.

`buffer_picker_uses_names_and_project_relative_or_absolute_paths` in
`src/app/tests/editing_and_buffers.rs` edits one of two open files and checks
that the edited row carries `[+]` while the clean row stays unmarked. The
subsequent destination-list unification builds buffer-list rows through
`buffer_destination_row` in `src/app/navigation_workflows.rs`, with the marker
in STATE; `buffer_picker_columns` continues to label Finder rows. The test
checks the STATE column and its semantic modified tint.

## Report

The buffer picker (`Space b b`) did not mark buffers with unsaved changes.
`buffer_picker_columns` in `src/app.rs` appended `[STALE]` and `[RO]` to a row
label but never `[+]`, so a modified buffer was listed exactly like a clean
one. The Navigator (`Space n`) and pane titles both showed `[+]` for the same
buffer.

A buffer with unsaved changes is expected to carry `[+]` in the buffer picker,
as it does in the Navigator and in its pane title. `[+]` and `[STALE]` can both
apply to one buffer (edited in the editor and changed on disk); both are
shown.

Reproduction:

1. Open `README.md` and make an edit without saving. The pane title shows
   `README.md [+]`.
2. Press `Space n`. The `README.md` row shows `[+]`.
3. Press `Esc`, then `Space b b`. The `README.md` row shows no `[+]`.
