---
title: "Explorer paste created an empty file instead of copying or moving the entry"
status: resolved
reported: 2026-07-31
resolved: 2026-07-31
legacy_commit: d9b11d4
---

## Resolution

Fixed by d9b11d4 ("Make explorer copy and cut use Helix registers").

`App::yank_value` and the register previously retained only rendered text, while
`DirectoryBuffer` kept filesystem identities inside one explorer buffer. After
navigation or a pane change, pasting therefore inserted an ordinary new row;
the filesystem planner correctly interpreted that identity-free row as an empty
file creation rather than as a copy of the selected entry.

The normal Helix `x`, `y`, `d`, and `p` commands now carry directory-transfer
metadata in Runyte's internal registers. Pasting attaches the source identity,
kind, copy-or-move intent, and source fingerprint to the destination row, so
`:w` or `:write` presents the existing confirmation popup without touching the
filesystem first. The same mechanism works after navigation in one pane,
between split panes, for multiple selections, and for recursive directories.
Same-directory copies retain the original snapshot identity, allowing normal
Helix editing to rename the pasted row before writing.

`FsPlan::build` resolves transfer paths against the destination explorer,
rejects self-recursive copies and sources that the same plan would destroy, and
`FsPlan::apply` revalidates every external source before performing any
operation. Source explorers are refreshed after a confirmed move when doing so
does not discard unrelated dirty edits. No Oil-specific key binding was added;
the interaction is Oil-like while the keymap remains Helix-shaped.

Behavior coverage in `src/app.rs` is
`explorer_yank_and_paste_copies_into_a_retargeted_pane_on_write`,
`explorer_paste_between_existing_rows_keeps_their_identities`,
`explorer_line_selection_copies_multiple_entries_with_helix_keys`,
`explorer_copy_can_be_renamed_in_the_same_directory_before_write`,
`deleting_the_original_of_a_same_directory_copy_becomes_a_rename`,
`explorer_delete_and_paste_moves_across_panes_on_write`, and
`a_pending_explorer_cut_can_follow_same_pane_navigation`. Safety coverage in
`tests/fs_plan.rs` is
`a_directory_transfer_cannot_copy_a_parent_into_its_descendant`,
`a_transfer_cannot_target_its_own_source_path`,
`a_transfer_copy_source_cannot_be_deleted_by_the_same_plan`, and
`a_changed_transfer_source_aborts_before_any_operation`. The full `cargo test`,
`cargo fmt --check`, and `cargo clippy --all-targets -- -D warnings` gates pass.

Known limitation: filesystem identity metadata is local to Runyte's internal
registers; the operating-system clipboard remains text-only. Moves across
filesystem boundaries still depend on `rename` and report an apply error when
the operating system cannot perform one atomically.

## Report

The existing copy and cut flow in the explorer was too complex. It should
behave closer to oil.nvim: copying or cutting a file and pasting it elsewhere,
in the same pane or another one, should actually copy or move that file. The
`I` and `Ctrl-s` bindings should not be part of it. Edits should be confirmed
with `:w` or `:write`, through the existing confirmation prompt.
