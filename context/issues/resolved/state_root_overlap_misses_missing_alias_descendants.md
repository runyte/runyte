---
title: "Missing state directories conceal overlap through symlink aliases"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 4035a9a
---

## Resolution

Commit `4035a9a` (`fix(storage): resolve aliases above missing state directories`).

On Unix, comparable_path resolves the longest existing ancestor before examining the unresolved suffix. This exposes containment through aliases even when several descendants have not been created. A subsequent independent review found that the suffix must also account for directory creation: create_dir_all creates an absent component before traversing a following parent component. The comparison now walks that prospective suffix in order, resolving existing symlinks again when parent traversal re-enters an existing directory. Initialization rejects overlap before creating runtime state; distinct future roots remain valid. This changes only storage-overlap comparison, not the configured path passed to creation or ordinary document path identity.

Coverage: state_root_overlap_resolves_aliases_above_missing_descendants in src/project_root.rs checks both containment directions, rejection without directory creation, and a separate valid future root. state_root_overlap_predicts_parent_components_after_directory_creation in the same file covers absent components followed by parent traversal, aliases before and after the unresolved segment, reverse overlap, and successful creation of a separate future directory without rewriting its configured path. Both regressions failed before their respective fixes; all 19 project_root tests pass on Linux.

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

A further reproduction reserves an existing temporary `reserved` directory and
initializes a state root at `missing/../reserved/runtime`. Comparing the
unresolved suffix verbatim accepts the path, but `create_dir_all` first creates
`missing`, traverses its parent, and then creates `reserved/runtime` inside
per-user storage. `missing/../alias/runtime`, where `alias` points to `reserved`,
has the same problem. An alias before the missing segment also matters:
`nested-alias/missing/../../runtime`, with `nested-alias` pointing to
`reserved/inside`, reaches `reserved/runtime`. Validation must predict these
creation semantics in both containment directions and reject them before any
directory is created. A separate path such as
`created/../separate/not-created/yet` remains valid; initialization must still
use the authored path and create `created` as part of normal directory creation.
