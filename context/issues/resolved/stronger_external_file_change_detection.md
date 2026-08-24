---
title: "External file changes could evade the save conflict guard"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: 297b10e
---

## Resolution

Commit 297b10e (`Strengthen external file change detection`) fixed the conflict guard. `DiskState` had represented a loaded file using only its length and modification time, and loading the text separately from that metadata left additional check/use windows. It now captures the text and baseline from one open handle and records a content digest, stable file identity where available, and access-control state.

Saving now carries an explicit expected-state, no-replace, or force policy into the atomic replacement operation. Supported platforms use native atomic exchange or replacement primitives so Runyte can validate the exact displaced object rather than trusting a pathname checked earlier. A mismatch is rolled back or retained at a reported recovery path. The protocol also detects symlink retargeting, concurrent permission or ACL changes, and a destination created during save-as. After replacement, the installed content is verified before the buffer is marked clean.

Tests in `src/buffer.rs` cover the behavior:

- `saving_detects_a_same_size_rewrite_with_a_preserved_timestamp`
- `atomic_save_rechecks_the_destination_after_writing_its_temporary`
- `a_post_commit_change_does_not_mark_different_buffer_text_clean`
- `save_as_does_not_replace_a_target_created_during_temporary_write`
- `retargeting_a_symlink_during_save_changes_neither_target`
- `tightening_permissions_during_save_is_not_overwritten`

Known limitation: ordinary non-force saves safely refuse on Unix platforms without a supported atomic exchange primitive, and on filesystems or kernels that reject the required operation. An explicit force save remains available when the user intentionally accepts that tradeoff.

## Report

The guard against overwriting externally changed files compared only file length and modification time.

Same-length rewrites on filesystems with coarse timestamp resolution, metadata-preserving tools, or replacements whose modification time was restored could compare equal to the recorded `DiskState`. A later normal save then overwrote the external contents without showing the conflict that the guard promised to detect. There were also check/use windows between reading or comparing metadata and accessing the path again.

The loaded baseline needed to retain a content digest and, where available, stable file identity. Save needed to compare the current disk content or digest as part of the atomic-save protocol before replacing it. Tests needed to perform a same-size rewrite with a preserved modification time and verify that ordinary save refused it.

Relevant code: `src/buffer.rs` in `DiskState`, `Buffer::open`, and `Buffer::save`.
