Filesystem-plan confinement is vulnerable to symlink and directory-replacement races between preflight and application.

The plan canonicalizes its root and checks target parents during preflight, but later move, create, copy, trash, and delete operations reopen string paths. Another process can replace the root or a checked parent directory with a symlink after validation and before use. The subsequent operation can then land outside the tree the person confirmed or delete newly substituted contents. `parent_is_ready` also follows symlinks through `is_dir`, so it is not a sufficient authorization check.

Operations should be anchored to an opened root directory and performed descriptor-relative with beneath and no-follow semantics. On Linux this can use `openat2` constraints; other Unix platforms need a component walk using `openat`, `fstatat`, `renameat`, and `unlinkat` equivalents while verifying identities at use time. Race tests should swap a checked parent and prove no operation escapes or affects replacement contents.

Relevant code: `src/fs_plan.rs` in plan construction, preflight, `parent_is_ready`, and operation application.

## Analysis

This is a medium-severity local vulnerability with potentially high-impact consequences. Exploitation requires a concurrent process with filesystem access under the same account to replace a checked entry or parent at the right point while a confirmed explorer plan is being applied. It is not directly remotely exploitable.

The timing requirement makes accidental triggering uncommon, although directory synchronizers and concurrent scripts can produce related races. A deliberate same-user process can make the race practical. A successful race can redirect creates, copies, or moves outside the directory the person reviewed, disclose copied data, or cause unintended movement or deletion. Existing lexical confinement, canonicalization, and preflight symlink checks stop ordinary traversal and obvious symlinks, but they cannot authorize a later pathname use after the checked component has been replaced.

An attempted localized fix showed that pinning only the root or parent directory is insufficient. Source and target leaves must also remain tied to the exact reviewed objects through staging, validation, publication, rollback, recursive copy and deletion, and trash handling. Intentional `..` sibling destinations further prevent treating the explorer root as a simple `RESOLVE_BENEATH` boundary. The default trash workflow exposes only ambient pathname operations, while the necessary no-follow, no-replace, and object-identity primitives differ across Linux, Apple platforms, other Unix systems, and Windows.

## Deferral

Fixing this correctly requires a broader capability-based, platform-specific filesystem layer rather than additional checks inside `FsPlan`. That layer needs stable directory capabilities, descriptor- or handle-relative component walks, atomic no-replace publication, exact-object staging and rollback, recursive operations that never follow substituted entries, and a safe platform trash abstraction. Unsupported platforms must have an explicit product policy instead of silently falling back to ambient paths or losing core explorer behavior.

The issue is deferred until that filesystem capability boundary is designed. A narrow patch is not being retained because it would either leave exploitable check/use windows or introduce substantial editor regressions, particularly for trash deletion and non-Linux platforms.

## Scoped data-safety work

The [filesystem-plan data-safety plan](../../plans/active/PLAN_FS_PLAN_DATA_SAFETY.md)
records the 2026-09-05 decision to implement a narrower set of guarantees:
atomic destination collision protection during staging and publication,
rollback that preserves concurrently recreated source entries and retains
originals for recovery, and ownership-aware temporary-copy cleanup. These
mitigations are implemented with Linux regression tests and canonical coverage
at 91.67%; native macOS validation remains pending in the active plan.

This scope supersedes the earlier rejection of narrow patches only for those
independently testable data-loss protections. It does not claim to resolve
symlink confinement or source replacement races. The broader capability
boundary, including native trash and recursive operations under hostile
substitution, remains deferred. Completion of the data-safety plan must leave
this issue deferred and record the implemented mitigations and remaining
limitations here.
