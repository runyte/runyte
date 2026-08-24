---
title: "File saves can truncate the only on-disk copy before completion"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: ce5f51e
---

## Resolution

Commit ce5f51e (Save buffers with atomic replacement) replaced the direct fs::write calls in Buffer::save and Buffer::save_as with a same-directory atomic-write path. The old functions opened the destination with truncation semantics, so a short write, I/O error, or process loss could destroy the prior complete file before the replacement was available.

The new path securely creates a private sibling temporary, writes and flushes the complete buffer, restores applicable access metadata, atomically installs it, and syncs the containing directory where the platform supports that operation. Existing Unix files retain UID, GID, mode bits, and supported native access ACLs. Existing Windows files use ReplaceFileW so DACLs and other replacement metadata are merged; new temporaries use a protected owner-only DACL. Windows partial replacement failures retain the complete temporary and a cryptographically named original-file backup instead of allowing cleanup to delete the only recoverable copy.

Symlink saves resolve and replace the link target while leaving the link itself intact. The save path also explicitly verifies write access to an existing destination so directory rename permission cannot bypass a read-only file. If replacement commits but backup cleanup or metadata/directory sync fails, SaveOutcome::CommittedWithWarning reconciles the buffer path, clean state, syntax, LSP, and Git integrations before surfacing the durability warning.

Tests in src/buffer.rs cover the behavior:

- a_failed_temporary_write_leaves_the_destination_intact
- a_retained_replacement_survives_automatic_temporary_cleanup
- a_post_commit_sync_error_keeps_save_state_aligned_with_disk
- a_post_commit_sync_error_still_adopts_a_save_as_path
- saving_through_a_symlink_preserves_the_link_and_target_permissions
- a_new_save_temporary_is_private_before_contents_are_written
- atomic_save_does_not_bypass_a_non_writable_destination
- atomic_replacement_restores_special_permission_bits_after_writing
- atomic_replacement_preserves_a_posix_access_acl

Known limitation: Unix targets without one of the implemented native ACL mechanisms refuse atomic replacement of an existing file rather than risk silently dropping its access controls. The Windows-only native path was target-compiled in isolation because this environment lacks the MinGW C headers required to cross-build Runyte's statically linked grammar dependencies.

## Report

Normal file saves wrote directly to the destination with fs::write.

Because the destination was truncated before the new contents were completely written, ENOSPC, an I/O error, process termination, or power loss could leave the only on-disk copy empty or partial. Runyte could correctly return a save error and retain a dirty in-memory buffer, but that buffer was then the only complete copy and would disappear if the process exited.

Both ordinary save and save-as needed to use a securely created temporary file in the destination directory, write and sync it according to the editor's durability policy, preserve applicable permissions, atomically rename it into place, and sync the parent directory where supported. Editing through a symlink needed to preserve its established behavior. Failure-injection tests needed to demonstrate that a short or failed write left the prior destination intact.

Relevant code was src/buffer.rs in Buffer::save and Buffer::save_as.
