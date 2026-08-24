---
title: "Every directory visited with the explorer leaves its own buffer behind"
status: resolved
reported: 2026-07-30
resolved: 2026-07-31
legacy_commit: 4a0c6b3
---

## Resolution

Fixed by 4a0c6b3 "Give each pane one explorer instead of one per directory".

`open_file` reused whatever buffer had a matching path and otherwise opened a
new one, which is right for files and wrong for directories: walking a tree
left one buffer per directory visited, and `Space b` listed the whole tour.

The report asked whether this would break copying and moving between
directories, and asked to be told before any change. The explorer retargeting
did not break either operation: at the time of the original fix, moves were
planned by editing paths inside one directory buffer, while cross-directory
copy did not exist at all. Nothing that worked before stopped working. The
reporter chose one explorer *per pane* over one globally, so two panes can
still show two directories side by side.

A later follow-up added that missing operation without changing explorer
ownership. Desired paths may use `..`, so a confirmed plan can create, move,
or recursively copy into a neighboring directory. A subsequent interaction
fix put that machinery beneath the normal Helix register commands: `x y` or
`x d` captures filesystem identities, and `p` in an explorer in the same or
another pane plans the corresponding copy or move directly. The planner still
rejects symlinks in every target parent, including parents above the explorer
root, and copies through a temporary sibling so a failed recursive copy does
not leave a partial final target.

A `Pane` now records the one directory buffer it browses with, and navigation
retargets it through a new `Buffer::retarget_directory` rather than opening a
second. Retargeting drops the undo history along with the listing: the two
directories share no text, and an undo across the boundary would restore
entries that were never in the directory now on screen.

Three ownership rules keep the count honest. A split starts with no explorer of
its own and takes one on its first navigation, so navigating in one split
cannot move the other's directory. `switch_buffer` adopts a directory buffer it
lands on, unless another pane is browsing with that buffer, in which case the
pane gives up its own reservation and takes a fresh explorer next time — a pane
that walks away must not keep reserving what it left, or nothing could ever
adopt it. And because buffers are never removed from `App::buffers`, a closed
pane's explorer is adopted by the next pane that needs one rather than
orphaned; `claimed_by_another_pane` counts displaying as claiming, not just
reserving, which is what the startup explorer needs before it has navigated
anywhere.

`directory_views` moved from being keyed by buffer index to being keyed by
path. One buffer now stands for every directory a pane has visited, so the
buffer can no longer say which remembered cursor row belongs to which listing.

Two consequences of retargeting had to be handled rather than accepted.
Unsaved edits to a listing are destroyed by it, so navigating away from a dirty
explorer asks before discarding, exactly as refresh already did;
`directory_reload_confirmation` grew a destination so confirming resumes the
blocked navigation instead of merely re-reading. And retargeting replaces the
whole text outside the transaction system, where nothing remaps offsets, so
jumps into a replaced listing are retired through `JumpList::forget` — a method
that had existed for this case and never had a caller — and an in-place
retarget records no jump of its own, since there is nothing left to go back to.

Tests: `directory_navigation_retargets_one_buffer_and_preserves_each_view`,
`navigating_away_from_a_dirty_explorer_asks_before_discarding`,
`each_pane_browses_with_an_explorer_of_its_own`,
`switching_to_a_directory_buffer_hands_over_the_panes_explorer`,
`switching_to_an_unclaimed_directory_buffer_adopts_it`,
`retargeting_an_explorer_retires_jumps_into_it`, and
`a_closed_panes_explorer_is_adopted_instead_of_orphaned`, all in `src/app.rs`.
Follow-up copy coverage is in `tests/directory_buffer.rs` as
`pasting_and_repathing_an_entry_produces_a_copy`, and in `tests/fs_plan.rs` as
`a_repeated_identity_copies_a_file_outside_the_explorer_root`,
`copied_directories_include_nested_files`,
`a_moved_entry_can_be_copied_from_its_final_path`, and
`escaping_the_root_does_not_allow_a_symlink_target_parent`.
The register interaction is covered in `src/app.rs` by
`explorer_yank_and_paste_copies_into_a_retargeted_pane_on_write`,
`explorer_paste_between_existing_rows_keeps_their_identities`,
`explorer_line_selection_copies_multiple_entries_with_helix_keys`,
`explorer_copy_can_be_renamed_in_the_same_directory_before_write`,
`deleting_the_original_of_a_same_directory_copy_becomes_a_rename`,
`explorer_delete_and_paste_moves_across_panes_on_write`, and
`a_pending_explorer_cut_can_follow_same_pane_navigation`.
Transfer safety is covered in `tests/fs_plan.rs` by
`a_directory_transfer_cannot_copy_a_parent_into_its_descendant`,
`a_transfer_cannot_target_its_own_source_path`,
`a_transfer_copy_source_cannot_be_deleted_by_the_same_plan`, and
`a_changed_transfer_source_aborts_before_any_operation`.

Known limitation: `directory_views` grows one entry per directory visited and
is never pruned, so a long session browsing a large tree keeps a small record
of every directory it saw. Panes are not garbage-collected either — a pane that
walks away from its explorer without any other pane needing one leaves that
buffer in the list until something adopts it.

## Report

Traversing directories with the explorer (`Space e` or `:explorer`) and then
opening the buffer list with `Space b` showed every visited directory as its
own buffer. Only one explorer buffer should be kept, and moving to another
directory should update its contents in place.

Whether that still supports copying and moving items between directories was
an open question at the time of the report.
