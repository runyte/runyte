---
title: "A stopped workspace displayed a different, shorter ID"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: 1e476e4
---

## Resolution

Commit `1e476e4` (`Keep workspace IDs stable after hosts stop`) fixed the
identity mismatch. `workspace::catalog::refresh` was independently hashing
recent workspace paths and truncating the result to 16 characters, while
`LocalEndpoint` used the transport's 32-character identity for running hosts.
The transport identity derivation is now shared within the workspace module,
and both endpoint construction and stopped catalog rows call that one helper.
This keeps the full ID stable across host state changes without changing exact
ID, unambiguous prefix, name, or directory selector behavior.

Tests covering the behavior:

- `workspace::catalog::tests::stopped_workspace_id_matches_the_running_endpoint_identity`
  in `src/workspace/catalog.rs`
- `workspace::catalog::tests::known_selector_paths_use_the_supplied_editor_directory_and_ids_and_names_stay_exact`
  in `src/workspace/catalog.rs`

## Report

Running workspace hosts displayed the transport's 32-character workspace ID,
but catalog rows reconstructed for stopped workspaces truncated the same
stable hash to 16 characters. A workspace's displayed full ID therefore
changed solely because its host stopped, contrary to the stable-ID contract,
and the shorter value had a greater collision risk.

Stopped catalog rows needed to derive IDs with the same width and algorithm as
`LocalEndpoint`, so running and stopped representations of one workspace used
the same identity.
