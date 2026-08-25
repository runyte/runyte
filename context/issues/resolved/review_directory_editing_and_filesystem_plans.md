---
title: "Directory projections and confirmed filesystem plans had unsafe edge cases"
status: resolved
reported: 2026-08-25
resolved: 2026-08-25
commit: 4fab0a3
---

## Resolution

Commit `4fab0a3` (`Harden explorer filesystem plans`) fixed five defects found
by the directory-editing and filesystem-plan review.

`DirectorySnapshot::read_with` rejected control characters but still admitted
filenames ending in whitespace, even though `DirectoryBuffer::parse_line`
removes trailing whitespace as editor-only syntax. An unchanged projection
could therefore rename or delete a real whitespace-ending entry. Conversely,
`parse_line` allowed an edited row to create an internal control-character
name which the next snapshot refused to open. The snapshot now refuses
included whitespace-ending names before rendering, and planning refuses
internal control characters before a confirmation opens. Existing non-UTF-8
and control-character refusal remains fail-safe, and filtered hidden entries
remain outside the projection and plan.

`SourceFingerprint` previously described only the source entry itself using
kind, length, modification time, and a possible link target. A change below a
nested directory did not necessarily change the top directory's metadata, so
a confirmed recursive move, copy, or delete could apply to a different tree.
Plan preparation now records every descendant without following symlinks, and
application compares the complete tree before performing any operation. Unix
device, inode, mode, owner, and change time strengthen both shallow and
recursive source identity checks. A source that cannot be captured remains
representable in a plan for explorer reconciliation, but its absent
fingerprint makes application fail closed.

`rollback_staged` used `Path::exists` to decide whether a staged source still
needed restoration. That follows links and returns false for a dangling
symlink, leaving a renamed link under an internal temporary name when a later
step failed. Staging and rollback now use `symlink_metadata`, so dangling links
are treated as entries and restored. Temporary-name selection uses the same
no-follow existence check and reports inspection failures instead of assuming
the name is free.

`DirectoryListings` originally keyed reuse by the requested path and directory
modification time. A retargeted directory symlink could therefore return its
old target's names when timestamps happened to match. Canonicalizing the cache
key fixed that alias case but an independent review found that a different
directory renamed into the same canonical pathname could still reuse the old
listing. Cache reuse now requires the canonical target, modification time, and
Unix device/inode identity to remain equal. The volatile timestamp window and
bounded eviction policy are otherwise unchanged.

Finally, `FsOperation::description` omitted the directory marker from rename,
move, copy, and delete rows. The confirmation could not distinguish a
recursive directory operation from the corresponding file operation. Review
text now appends `/` to both ends of directory operations while continuing to
render directly from the typed operations that application executes.

Regression coverage is provided by:

- `tests/directory_buffer.rs::a_directory_with_a_trailing_whitespace_filename_is_refused_before_rendering`
- `tests/directory_buffer.rs::an_edited_control_character_name_is_rejected_before_confirmation`
- `tests/fs_plan.rs::changes_below_a_nested_directory_stale_a_confirmed_recursive_delete`
- `tests/fs_plan.rs::a_dangling_symlink_is_restored_when_a_later_plan_step_fails`
- `tests/fs_plan.rs::directory_operations_are_marked_as_recursive_in_the_review_text`
- `src/directory_listing.rs::tests::a_retargeted_directory_symlink_cannot_reuse_the_old_targets_listing`
- `src/directory_listing.rs::tests::a_replaced_directory_cannot_reuse_the_previous_objects_listing`

Known limitation: descriptor-relative protection against concurrent symlink or
directory replacement remains deferred in
`context/issues/deferred/fs_plan_symlink_race.md`; this change does not claim to
close that check/use window. Capturing a confirmed directory source is linear
in the size of its tree. Cross-filesystem moves still fail without applying
the move when the platform rename cannot cross devices. Directory cache object
identity uses device/inode values on supported Unix platforms and retains the
mtime/window fallback elsewhere.

## Report

Editable directory projections and the confirmed filesystem plans they
produce required a focused hardening review. The review was proactive rather
than based on one known defect, and changes were limited to confirmed
problems. The deferred `fs_plan_symlink_race` capability-layer work was
explicitly outside scope.

The primary review surface was `src/directory_buffer.rs`,
`src/directory_listing.rs`, `src/fs_plan.rs`, `src/path_safety.rs`, relevant
file workflows, and their tests. The audit covered hidden entry identity,
refreshes after external changes, unusual and non-UTF-8 names, rename and move
cycles, overwrite conflicts, project-path containment, symlinks, trash and
permanent deletion, cross-filesystem moves, partial application, rollback
behavior, stale confirmations, cache invalidation, and consistency between the
displayed plan and executed effects.

Every confirmed defect required temporary-directory regression coverage.
Confirmation and containment rules were not to be weakened for an edge case.
The completed implementation also required independent code review against
the full diff and relevant invariants, incorporation or technical disposition
of every actionable finding, targeted tests, `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`.
