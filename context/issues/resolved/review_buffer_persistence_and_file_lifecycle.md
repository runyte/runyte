---
title: "Path aliases could split file buffers and binary files could enter text buffers"
status: resolved
reported: 2026-08-25
resolved: 2026-08-25
commit: cc894c9
---

## Resolution

Commit cc894c9 (Harden file buffer lifecycle) fixed three confirmed defects in
file-backed buffer ownership and classification.

Interactive opens and persistent-host opens compared stored path spellings,
while startup only canonicalized complete existing paths. `path_identity` now
defines the shared buffer identity: it canonicalizes existing paths, follows a
dangling leaf symlink to its intended missing target, resolves existing parent
aliases for missing leaves, and deliberately retains an unresolved component
sequence such as `missing/../file`. Startup, interactive opens, and atomic host
requests deduplicate on that identity. A newly read buffer is admitted only if
the requested path still has its pre-read identity, so an external symlink
retarget cannot make two opens converge after the initial ownership check.

`App::save_buffer` previously did not reserve a destination against every
other live buffer. Forced save-as could therefore overwrite a file owned by a
different live buffer, while a later external retarget could leave two buffers
owning the same identity. Saves now reject every other live owner before
writing. The approved identity is carried into the atomic-write path and
checked before creating the replacement and again after the temporary has
been synced but before it is installed. `Buffer::save_as` also recognizes an
alias of its own current file, so the ownership hardening does not reject a
safe write through that file's symlink.

`read_text_and_state` previously relied on `read_to_string` after the bounded
eight-kilobyte open probe. Invalid UTF-8 beyond the probe produced a generic
open error, while NUL bytes beyond it were admitted as editable text; reload
had no binary classification at all. The final complete read now classifies
the entire byte sequence before constructing text and returns a typed binary
error. Interactive and startup opens route that late result to the existing
external-program prompt, and reload refuses it without changing text,
revision, dirty state, disk state, or undo history.

Regression coverage:

- `src/path_safety.rs`:
  `unresolved_parent_components_are_not_lexically_cancelled`,
  `a_dangling_symlink_has_the_identity_of_its_missing_target`, and
  `a_missing_file_under_a_symlinked_parent_uses_the_real_parent_identity`.
- `src/app/tests/commands.rs`:
  `a_late_binary_startup_target_reaches_the_external_program_prompt`,
  `a_file_open_rejects_a_symlink_identity_changed_after_preflight`,
  `missing_launch_targets_below_a_symlinked_parent_share_one_buffer`, and
  `binary_bytes_beyond_the_probe_still_use_the_external_program_prompt`.
- `src/app/tests/navigation_and_files.rs`:
  `opening_a_symlink_alias_reuses_the_live_file_buffer`,
  `force_save_as_refuses_a_path_owned_by_another_live_buffer`, and
  `saving_refuses_a_second_buffer_that_converged_on_the_same_file`.
- `src/buffer.rs`:
  `reload_rejects_binary_replacement_without_changing_live_state`,
  `save_as_accepts_an_alias_of_the_buffers_current_file`, and
  `an_identity_checked_force_save_rejects_a_retargeted_symlink`.
- `src/workspace/host.rs`:
  `one_host_request_deduplicates_resolved_file_aliases`.

Known limitation: file operations remain pathname based. The identity checks
make a concurrent replacement fail safely in the covered windows, but they do
not provide descriptor-relative filesystem capabilities that could eliminate
every race with a hostile process continuously replacing ancestor paths.

## Report

A focused review of opening, retaining, reloading, saving, renaming, sharing,
and closing file-backed buffers found defects in path ownership and complete
binary classification.

Different spellings of one resolved file, including symlink aliases, could be
opened as independent buffers after startup and through the persistent host.
Missing paths beneath a symlinked parent and dangling symlink leaves also
lacked a stable shared identity. Those buffers could retain divergent dirty
text for one filesystem destination. Save-as, including `:write!`, could then
target a path already owned by another live buffer, and external symlink
retargeting between approval and I/O could invalidate the ownership decision.
Repeated resolved spellings are expected to share one live buffer in both
standalone and persistent modes, unresolved `..` traversal must retain its
filesystem meaning, and a save must not silently take over another live
buffer's file identity.

Binary detection initially inspected only the first eight kilobytes. The
complete file read did not repeat the NUL and UTF-8 classification, so binary
bytes beyond that probe either entered an editable buffer or escaped the
external-program workflow as a generic read failure. Reload could likewise
replace a text buffer with NUL-containing contents. The complete bytes must be
validated before any open or reload mutates live editor state; a refused
reload must preserve text, revision, dirty state, disk state, and undo history.

The review also covered atomic-save failure handling, dirty-state accuracy,
external-change detection, forced-save semantics, metadata and permission
preservation, line endings, partial I/O, shared-buffer close protection,
wait-owned buffers, and cleanup after failure. No additional defect requiring
a safe local change was confirmed in those areas. Regression tests use
temporary directories and do not write runtime state into the repository or a
user configuration or cache directory.
