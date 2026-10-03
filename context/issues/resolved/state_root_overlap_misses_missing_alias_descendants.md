---
title: "Missing state directories conceal overlap through symlink aliases"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 4035a9a
---

## Resolution

Commit `4035a9a` (`fix(storage): resolve aliases above missing state directories`).

On Unix, comparable_path now resolves the longest existing ancestor before appending the unresolved suffix. This exposes containment through aliases even when several descendants have not been created, while preserving the spelling and semantics of unresolved components. Initialization rejects overlap before creating runtime state; distinct future roots remain valid.

Coverage: state_root_overlap_resolves_aliases_above_missing_descendants in src/project_root.rs checks both containment directions, rejection without directory creation, and a separate valid future root. The regression failed before the fix; all 18 project_root tests pass.

## Report

On Unix, runtime-state overlap validation can accept a path inside reserved
per-user storage when the configured path has multiple nonexistent components
below a symbolic-link alias. `project_root::comparable_path` canonicalizes only
the complete path or its immediate parent. If both are absent, it compares the
original alias spelling with the canonical reserved path and misses their
overlap.

For example, create a temporary `reserved` directory and an `alias` symlink to
it. Reserve `reserved` as per-user storage, then validate
`alias/not-created/yet` as the workspace state root. The validator accepts it,
although creating that directory places runtime state under `reserved`.
The Windows implementation already walks to the longest existing ancestor.

Resolve the existing ancestor before comparing both containment directions,
preserving supported symlinks and absent future state directories. A fixture
must inject all paths under temporary storage and verify that overlapping
paths are rejected before initialization creates them. Also cover a distinct
future directory that remains valid. This is a deterministic path-resolution
error, separate from concurrent filesystem-plan substitution races.
