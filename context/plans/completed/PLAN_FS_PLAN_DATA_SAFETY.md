# Filesystem-plan data safety

Status: completed 2026-09-05; Linux and native macOS validated

Created: 2026-09-05

Implementation started: 2026-09-05

Related issue: [Filesystem-plan symlink races](../../issues/deferred/fs_plan_symlink_race.md)

## Decision and scope

Prioritize accidental data loss when another editor, synchronizer, build tool,
or script changes directory entries during a confirmed explorer plan. Implement
atomic destination collision protection on Linux and macOS, preservation of
staged originals when rollback cannot restore them, and ownership-aware copy
cleanup. These requirements are independent of full filesystem confinement.

The broader issue remains deferred. This work does not establish a security
boundary against a hostile process running as the same user, freeze source
contents, or guarantee exact-object deletion under concurrent replacement.
Parent-directory and source-leaf substitution remain possible with pathname
operations, including when the competing process is non-malicious. Describe
the guarantees by operation, not by whether a race was intentional.

Current comparable editor implementations inform this prioritization, not a
claim that a vulnerability is harmless or that no editor protects against it.
VS Code validates moves before pathname-based rename, and Oil.nvim performs
pathname-based recursive operations. The inspected implementations do not
provide the complete capability boundary requested by the deferred issue.
See the [VS Code filesystem provider](https://github.com/microsoft/vscode/blob/main/src/vs/platform/files/node/diskFileSystemProvider.ts)
and [Oil.nvim filesystem implementation](https://github.com/stevearc/oil.nvim/blob/master/lua/oil/fs.lua),
inspected on 2026-09-05. These are moving upstream references, not a security
audit or a prerequisite for this plan's acceptance.

## Required behavior

1. A create, move, rename, copy publication, or rollback must fail if its
   destination entry exists at the instant of installation. Files, empty and
   nonempty directories, symlinks, and dangling symlinks all count as entries.
   A preflight check alone does not satisfy this requirement.
2. Temporary-name collisions must never overwrite or trigger cleanup of the
   competing entry. Retry only allocation collisions, with a bounded limit;
   a collision at a user-selected destination stops the plan.
3. If a source name is recreated after staging, rollback leaves that entry
   untouched and retains the staged original. The error identifies the
   original location, retained location, and reason restoration failed.
4. Failure reports distinguish completed operations from originals retained
   for recovery and incomplete copy artifacts. A retained original is neither
   a successfully applied move nor a successfully restored source.
5. A cleanup failure must not hide the initial failure. Uncertain ownership
   results in a retained artifact and a diagnostic, not recursive deletion.
6. Keep rename cycles, moves into newly created or renamed directories,
   sibling destinations such as `../destination`, cross-explorer transfers,
   copied symlinks, native trash, and explicit permanent deletion working.
   Preserve the existing refusal of cross-filesystem moves; adding copy-and-
   delete fallback is outside this work.

These are destination-entry guarantees at the directory resolved by the
operation. They do not prove that a replaced parent still names the directory
reviewed in the confirmation.

## Current failure points

All of the following are in `src/fs_plan.rs`:

| Site | Current behavior | Required change |
| --- | --- | --- |
| `apply_with_trash`, move staging | Checks a generated name, then calls `fs::rename` | Atomically claim an absent staging destination |
| `PendingStep::Finalize` | Calls `fs::rename` after preflight | Publish without replacement |
| `copy_path` | Checks target, copies to a checked temporary name, then renames | Exclusively own staging and publish without replacement |
| `copy_entry`, regular files | Uses `fs::copy`, which can overwrite a destination | Open destination with exclusive creation and copy through the owned handle |
| `remove_copy` | Deletes by temporary pathname without an ownership record | Clean only attributable artifacts; retain ambiguous contents |
| `rollback_staged` | Renames over the old source name | Restore without replacement; retain and report conflicts |
| `preflight` | Ignores every target metadata error | Treat only `NotFound` as absence; propagate other errors |

File creation already uses `create_new(true)`, and directory creation already
uses `create_dir`. Keep those exclusive operations. Existing source
fingerprints, parent validation, and operation dependency ordering remain
useful conflict checks, but must not be described as race-proof authorization.

## Implementation boundary

### Atomic rename without replacement

Add a private platform module, proposed as `src/fs_plan/platform.rs`, exposing
one narrow operation such as `rename_noreplace(source, destination)`. Keep OS
calls and error translation here; `FsPlan` continues to own scheduling,
confirmation semantics, and reporting. Prefer the existing `libc` dependency
where it exposes the required calls. Validate path conversion and document
every unsafe call's preconditions.

| Platform | Implementation | Failure policy |
| --- | --- | --- |
| Linux | `renameat2` with `RENAME_NOREPLACE` | Preserve collision, unsupported-operation, cross-device, and ordinary I/O errors |
| macOS | `renameatx_np` with `RENAME_EXCL` | Same contract; support depends on the filesystem |
| Other targets | Return an explicit unsupported-operation result for this primitive | Never silently substitute an overwriting rename |

Linux documents both the exclusive rename flag and its filesystem support
requirements in [rename(2)](https://man7.org/linux/man-pages/man2/renameat2.2.html).
Apple documents exclusive rename and unsupported-filesystem errors in its
[rename manual source](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/man/man2/rename.2).
No newer optional Apple confinement flags are needed for this scope.

There is no check-then-rename, remove-then-rename, or hard-link-then-unlink
fallback. Do not add an overwrite override to the explorer confirmation.
Windows is not currently a supported release platform; keep builds coherent
without claiming a new Windows backend. Operations that do not need this
primitive need not be disabled on other targets.

Before user-data mutation, check that the required exclusive renames are
supported on each relevant filesystem. A bounded probe may use exclusively
created disposable entries in owned staging directories. Include a collision
probe that verifies both entries survive, and a successful rename probe.
For not-yet-created parents, account for the filesystem of their existing
ancestor and the plan's directory moves. Do not assume a single global
capability or silently skip a required probe. Real operation errors still
take precedence: probes cannot freeze mounts, permissions, or directory state.

If a primitive later becomes unavailable, stop, attempt only safe restoration,
and report any retained originals. A known unsupported filesystem must not
first consume deletes from a mixed plan. Detect cross-device move constraints
before staging where possible, and preserve the source on any later failure.

### Staging ownership and lifetime

Represent staged objects explicitly instead of `(FsOperation, PathBuf)` pairs.
Record the operation, original path, staging path, object identity, and state
such as staged, published, restored, or retained for recovery.

Allocate exclusive staging directories with restrictive permissions and a
bounded collision retry. Keep move staging on the existing staging filesystem
without broadening cross-device behavior; keep copy staging beside the final
destination so publication is on the same filesystem. Place the staged entry
inside its owned container. Directory creation itself establishes ownership;
an unoccupied-looking generated name does not.

Track identity separately from the existing content fingerprint. On Linux and
macOS, device, inode, and entry kind identify the object for best-effort
ownership checks. Rename and writes can change timestamps, so full fingerprint
equality is not a valid test that an artifact still belongs to this operation.
Identity checks are not an atomic authorization for a subsequent pathname use.

No destructor may unconditionally delete a staged original. Successful
publication or restoration removes only the now-empty owned container.
Retained originals survive error return, plan drop, editor shutdown, and later
plans unless explicitly recovered by the person. Do not put these originals
in `.runyte/`, a platform cache, or an automatically purged temporary directory:
they may be the only remaining copy of user data. Never sweep old staging
directories merely because their names match a prefix.

Recovery must be understandable from the error and retained paths. This phase
does not add an automatic recovery command, a crash journal, or a guarantee
that a diagnostic survives an abrupt process termination.

### Publication and rollback

Route move staging, move finalization, copy publication, and rollback through
the same exclusive primitive. A target vacated by this plan still must be
absent at publication time; prior approval does not authorize a replacement
that appeared afterward.

Preserve the create/finalize dependency scheduler and reverse staging rollback
order. After failure, restore every still-staged original that can be safely
restored, even if restoration of another entry fails. Never roll back a
completed move by overwriting its previous source. Keep the existing partial
application model: completed creates, moves, and deletes are not presented as
an all-or-nothing transaction.

If the staged object is missing or has an unexpected identity, report that
condition explicitly instead of treating it as a successful restoration.
If the original parent is missing, or restoration collides or fails, retain
the staged object and report the recovery location. A recreated original name
must not be unlinked, moved aside, or sent to trash to make rollback succeed.

### Copy construction and cleanup

Build the copy inside its owned staging directory. Open regular destinations
with exclusive creation, stream bytes through that handle, and apply the
intended permissions through the handle where supported. Preserve directory
permissions and symlink values, including dangling links. Apply restrictive
directory permissions only after their children have been copied.

Keep an ownership record for created artifacts, including partial copies.
On failure, cleanup must not touch an entry whose creation failed with a
collision. Check recorded identities before removing attributable artifacts,
never follow a replaced symlink, and use empty-directory removal when retiring
containers. If unexpected descendants or changed ownership make recursive
cleanup ambiguous, retain the affected copy tree and report it. Do not use an
unconditional `remove_dir_all` on a temporary name selected before allocation.

A failed publication may discard a demonstrably owned copy while leaving the
original source and competing destination intact. It must retain uncertain
artifacts. This cleanup policy limits ordinary collision damage; it still
does not close an adversarial check/use window after ownership validation.

### Editor reporting and compatibility

Extend the semantic failure/report values with structured recovery entries;
do not make the application parse error strings. Include original and retained
paths, artifact kind, and the reason recovery or cleanup could not complete.
Use explicit absolute paths in recovery diagnostics so sibling destinations
and later changes of working directory cannot make them ambiguous.

Update `apply_fs_confirmation` and `reconcile_applied_filesystem` in
`src/app/input.rs` to consume the distinction between applied operations and
retained artifacts. Keep unsaved buffers intact. A staged original awaiting
recovery must not cause an open buffer to be retargeted to an uninstalled
destination or silently adopted as the newly recreated source file.

Use the existing ERROR notification and notification buffer for recovery
details. A summary must not discard the full list of retained paths. Refresh
explorer state consistently with partial application and require a fresh plan
after a conflict; do not automatically retry the old confirmation. No bindings
or new confirmation surfaces are needed.

Keep `TrashBackend` and native trash behavior intact. Do not redirect ordinary
deletions into staging as part of this work: that would change trash restoration
paths and require the deferred trash design. Regress both trash and permanent
deletion in mixed plans using an injected temporary-directory trash backend.
The present order can leave confirmed deletions applied before a later
collision; preserve accurate reporting rather than promising reversibility.

## Delivery order

- [x] **1. Atomic operations and deterministic race seams.** Add the platform
  primitive, support/error policy, and a private per-application test hook or
  injected internal operations boundary. Test actual OS collision behavior on
  both supported platforms. Avoid a public testing API or global mutable hook.
  Implementation and native Linux/macOS checks are complete.
- [x] **2. Move staging, publication, and recovery.** Add owned staging records,
  exclusive rename at every move/rollback site, structured retained-original
  reporting, and minimum editor integration. Include rename-cycle regression
  coverage. This is the highest-priority user-data protection milestone.
- [x] **3. Copy ownership and cleanup.** Replace overwriting copy creation,
  publish exclusively, and make cleanup preserve ambiguous artifacts. Cover
  nested copies and links. Complete target metadata error handling.
- [x] **4. Editor validation and documentation.** Test partial-application
  reconciliation and recoverable errors, document the exact guarantees in
  `docs/user-guide.md`, and complete Linux/macOS validation and coverage checks.

Each implementation commit includes its behavior tests. Milestones 2 and 3
must each ship a complete failure path; do not temporarily make failed moves
unrecoverable or depend on a later UI commit to expose retained originals.

### Implementation record

Implementation commit: `6a1bd25` — `Prevent filesystem-plan overwrites and
preserve rollback originals`.

`src/fs_plan/platform.rs` provides exclusive rename on Linux and macOS with no
overwriting fallback. `src/fs_plan/staging.rs` owns restrictive, exclusively
created staging directories, validates artifact identities, probes filesystem
support, and conservatively cleans copied artifacts. `FsPlan` uses these
operations for staging, publication, and rollback. `ApplyReport::recovery`
records originals and artifacts requiring attention; application reconciliation
preserves unsaved source buffers and reports partial or recoverable failures
through ERROR notifications.

The macOS regular-file copy backend uses `fcopyfile` with
`COPYFILE_DATA | COPYFILE_METADATA` on the opened source and exclusively
created destination handles. This preserves resource forks, extended
attributes, ACLs, and native metadata behavior that the initial stream-copy
implementation lost relative to `std::fs::copy`. Native copy errors propagate
without a data-only fallback, and no later permission update changes the copied
ACL. The backend does not attempt APFS cloning; that performance optimization
is separate from metadata preservation.

`src/fs_plan/tests/macos.rs` covers metadata-preserving copies of regular files
and nested files, publication collisions with distinct competing metadata,
and native error propagation. These tests passed on native macOS during
milestone 4 validation.
`file_copy_uses_owned_handles_after_destination_name_is_replaced` in
`src/fs_plan/tests/mod.rs` checks on Unix that data and permission updates stay
on the owned descriptor when its pathname is replaced.

The implementation keeps the original plan scheduler and deletion order.
Recovery diagnostics remain subject to the existing notification storage and
retention bounds. Staging itself is never swept automatically. No new bindings,
native-trash replacement, or confinement guarantee were introduced.

Linux validation on 2026-09-05 passed `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`. The full suite
requires unrestricted local IPC: the sandbox rejected the unchanged Git
pipe-finalizer test with `Operation not permitted` and stalled a workspace
transport test; the suite passed outside that sandbox. The canonical
`cargo llvm-cov --locked --workspace` run passed with 91.67% total line coverage
(105,716 lines, 8,806 uncovered, including the macOS copy-backend follow-up),
above the enforced 89% floor.

Native macOS validation on 2026-09-05 passed `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, and the canonical
`cargo llvm-cov --locked --workspace` command. The host was macOS 26.6.2,
`aarch64-apple-darwin`, with Rust 1.97.1 and `cargo-llvm-cov` 0.9.0, at base
commit `61cb882` plus the milestone 4 test and documentation changes. Both
test runs passed 2,918 tests, with 33 existing ignored tests. They ran outside
the sandbox because it denies Unix-socket operations: the LSP persistent-host
test failed on that denial, and other host tests can return early for it.
The unrestricted run exercises those paths instead of accepting sandbox skips.
Total line coverage was 91.60% (106,514 lines, 8,947 uncovered), above the
unchanged 89% floor. `src/fs_plan/platform.rs` reached 100% line coverage.
The platform backend includes the metadata-preservation follow-up `a1f363b`
and macOS test-import correction `1b2f26d`.

The first native suite run also exposed two path-completion test assertions
that required an entire absolute temporary path to fit in a fixed 120-column
viewport. The macOS temporary-directory prefix exceeded that space.
`the_finder_path_assistance_is_titled_for_the_finder_and_carries_no_query_of_its_own`
and `a_hint_row_shows_the_entry_name_while_tab_still_completes_the_whole_spelling`
in `tests/path_completion.rs` now size their viewport for the path's display
width. No production behavior changed for milestone 4.

New deterministic regressions live in `src/fs_plan/tests/mod.rs`, including
`move_publication_collision_restores_original`,
`rollback_conflict_preserves_both_files_and_restores_other_originals`,
`copy_creation_collision_is_not_truncated_or_cleaned`,
`cleanup_retains_unexpected_children_and_original_failure`, and
`unsupported_rename_is_detected_before_mixed_plan_deletes`.
`src/app/tests/navigation_and_files.rs` adds
`filesystem_recovery_keeps_unsaved_source_and_replacement_protected` and
`filesystem_confirmation_retains_recovery_paths_in_an_error_notification`.
Milestone 4 extends the former to a partial report containing a completed
rename alongside a retained original: only the completed rename retargets its
open buffer, both unsaved file texts and the initiating explorer's edits
survive, and saving the retained original's buffer refuses the replacement on
disk. The latter forces two rollback conflicts and checks that both original
and retained paths remain in the notification buffer after another action
replaces the interaction-line summary.
The existing partial-application and dangling-symlink rollback tests in
`tests/fs_plan.rs` now inject interference through a temporary trash backend:
invalid filenames are correctly rejected during preflight and no longer
exercise application failure.

## Acceptance tests

Use temporary directories and deterministic injection immediately before the
real filesystem operation. No timing sleeps or probabilistic stress loop is
the sole regression test. Test hooks should be private and local to one apply
invocation; a module test can exercise private seams without expanding the
headless or public `FsPlan` contract.

| Scenario | Required assertion |
| --- | --- |
| Destination appears after preflight, before move publication | Competing bytes/identity survive; original is restored or retained and reported |
| Destination appears immediately before copy publication | Competing entry survives; source survives; cleanup respects ownership |
| File, empty directory, nonempty directory, symlink, dangling link at destination | Every applicable publication reports conflict without replacing the entry or affecting a link target |
| Temporary container or staged-entry name collides | Existing entry survives untouched; allocation retries within its bound or stops clearly |
| Old source name recreated before rollback | Replacement and staged original both survive; recovery entry names both paths |
| Multiple staged originals, one rollback conflict | Unblocked originals restore; every blocked original is retained and reported |
| Parent missing during rollback or injected restoration I/O error | Original stays in staging; error names retained path and cause |
| Missing or substituted staged object | Report uncertainty; do not declare restoration or clean the replacement |
| Copy creation collision, partial copy failure, or cleanup failure | Never truncate/delete the competing entry; preserve original error and report retained artifacts |
| Unexpected child added to copy staging before cleanup | Preserve the unexpected child and retain ambiguous tree |
| Unsupported exclusive primitive, invalid path, permission error, cross-device move | No unsafe fallback; preflight-known failures precede user-data mutation |
| Failure after successful operations in either deletion mode | Report precisely what applied and what remains recoverable |
| Confirmation error with an open unsaved source buffer | Text survives; buffer is not retargeted to the failed destination or rebound silently to a replacement |
| Plan drop and a later plan after blocked rollback | Retained original remains accessible and is not swept as stale temporary data |

Retain public behavior tests in `tests/fs_plan.rs`, especially
`a_rename_cycle_uses_temporaries_without_losing_either_file`,
`a_new_entry_can_be_created_inside_a_directory_the_same_plan_renames`,
`a_partial_failure_reports_exactly_what_was_applied`,
`a_dangling_symlink_is_restored_when_a_later_plan_step_fails`, the parent-relative
transfer tests, and `copying_a_symlink_preserves_the_link_instead_of_its_target`.
Place private race-seam tests under `src/fs_plan/tests/` and application tests
in `src/app/tests/navigation_and_files.rs` or a focused sibling test module.
Add new test names to the eventual implementation record.

Run before handing off Rust changes:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo llvm-cov --locked --workspace
```

Run platform-specific behavior on native Linux and macOS, not only through
cross-compilation. The canonical total line coverage must remain at or above
89% on affected first-class targets, following
[the coverage register](../../reference/test-coverage.md). Keep CI, badge, and
register synchronized if the floor is raised; do not lower it. Test unsupported
errors through injection as well as normal real-filesystem operation.
All storage and fake trash stay under test temporary directories. Never run a
test-created executable or touch personal trash, caches, or configuration.

## Completion and remaining deferral

All four milestones are complete. Native Linux and macOS validation passed,
and the user guide explains collision behavior, retained originals, recovery
notifications, and partial application. This plan is retained in `completed/`;
the plan index and the deferred issue link to this record.

Keep `fs_plan_symlink_race.md` in `issues/deferred/`, recording implemented
mitigations and their commit references without marking the full issue
resolved. If implementation is tracked as a separate focused open issue,
follow the repository's two-commit resolution procedure for that issue.

Remaining deferred work includes descriptor-relative component walks, root
and parent identity under replacement, source-leaf identity at mutation time,
recursive deletion/copy under substitution, and native trash confinement.
It requires a separate approved design with explicit platform and filesystem
support policy. Crash-consistent multi-operation transactions, automatic
recovery, and protection against concurrent writes through already-open
handles are also outside this plan.
