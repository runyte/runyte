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
