---
title: "Explorer file operations can become stuck on a stale directory snapshot"
status: resolved
reported: 2026-08-12
resolved: 2026-08-13
legacy_commit: da121d7
---

## Resolution

Commit `da121d7` (`Fix explorer filesystem plan reconciliation`) fixed three
interacting faults in explorer planning and reconciliation.

`DirectorySnapshot::matches_current` compared the content fingerprint of every
visible file, even when a plan did not operate on that file. Saving an ordinary
file after opening its parent explorer therefore made later, unrelated
directory edits fail as though the directory membership had changed. Snapshot
matching now compares entry paths and kinds, while
`FsPlan::operation_sources_unchanged` retains the stricter fingerprint check
for entries the plan will move, copy, rename, or delete.

`DirectoryBuffer::reconcile` could also transfer a deleted entry's identity to
an already-created row, changing a create-plus-delete edit into a rename and
moving the old file contents under the new name. It also treated every
same-sized edit as an in-place edit, so a whole-list reorder could attach
identities to the wrong rows. Reconciliation now distinguishes an originless
row that was already matched from a row that still needs an identity, restores
exact labels before falling back to row position, and retains row position only
for unresolved same-sized in-place edits.

Finally, `App::reconcile_applied_filesystem` preserved a dirty source explorer
after a move completed in another pane but left that explorer on its old
baseline. Its next write could only fail with `directory changed on disk`.
Dirty source explorers now advance their baseline past the exact removals
completed by the other pane while preserving unrelated local edits. This makes
multi-file cuts mixed with creations writable in sequence across panes.

Coverage is in:

- `tests/directory_buffer.rs`:
  `deleting_an_entry_does_not_turn_an_existing_new_row_into_a_rename` and
  `reordering_all_rows_in_one_transaction_keeps_their_identities`.
- `tests/fs_plan.rs`: `changes_to_an_unaffected_file_do_not_stale_the_plan`,
  `a_changed_file_still_stales_a_plan_that_moves_it`, and
  `one_plan_can_mix_multiple_creates_a_rename_and_multiple_moves`.
- `src/app.rs`:
  `a_cross_pane_move_rebases_other_source_edits_before_their_write`, alongside
  the existing same-pane, cross-pane copy, multi-copy, rename, directory, and
  move tests.

Known limitation: a dirty explorer is automatically rebased only when the
other confirmed plan removed entries that its text had already removed. An
external addition or rename into the same dirty listing still requires an
explicit refresh because preserving both versions would require a textual
merge.

## Report

File operations in the explorer sometimes failed with a message that the
directory had changed and that changes should be saved first. Saving was not
effective. The only known recovery was to exit Runyte with `:q!` and repeat the
file operations.

The failure was difficult to reproduce and no exact sequence was known. The
requested coverage included multiple file creation, renaming, directory
creation, moving multiple files, mixtures of those operations, operations in a
single pane, and moving or copying files across explorer panes.
