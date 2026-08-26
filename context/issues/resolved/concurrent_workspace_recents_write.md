---
title: "Catalog refresh can erase concurrently recorded workspaces"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: 4fc6c29
---

## Resolution

Fixed in commit `4fc6c29`, "Serialize workspace recents updates".

`refresh` in `src/workspace/catalog.rs` retained the recents snapshot it read
before inspecting running hosts and later passed that stale list directly to
`write_recents`. Because `record_recent_workspace_in` also performed an
unlocked read-modify-write of the per-user file, either process could replace
the catalog after reading an older version. A refresh delayed by host control
timeouts could therefore erase a row another Runyte process had recorded in
the meantime.

Both recording and refresh name writeback now use `update_recents`, which
acquires a dedicated adjacent `workspaces.lock`, re-reads the current catalog,
merges its change, and retains the lock through atomic replacement. Refresh
performs a three-way merge against its original snapshot: it preserves current
row order and newly added rows, and applies an inspected host name only when
the current name still equals the snapshot value. A concurrent name update
therefore wins over stale inspection as well.

`RecentFileLock` uses an exclusive Unix `flock` on only that dedicated lock
file, retries interrupted acquisition, rejects symlink traversal when opening
it, and enforces owner-only mode `0600` through the opened descriptor even
when the file already existed. Dropping the descriptor releases the lock, and
the kernel also releases it if a process crashes; the reusable lock file may
remain without leaving the catalog permanently locked. The potentially
blocking refresh transaction runs through Tokio's blocking worker pool.
Malformed recents still fail the locked re-read and are not replaced.

A later regression correction kept recents storage optional when its cache
root is unusable. `recent_file_in` now prepares the cache directory before
returning a recents path; if that directory cannot be created, catalog refresh
continues without stopped-workspace history, allowing the independent runtime
registry to keep running hosts discoverable. This fallback applies only to an
unusable cache root. Once the root is usable, a malformed `workspaces.json`
still produces an error and is left unchanged.

A later CI regression exposed a publication race in that fallback.
`LocalEndpoint::publish_metadata` wrote the endpoint's `endpoint.json` before
its registry rows, although startup and test clients treated that endpoint
metadata as the host-readiness marker. A session listing started in between
those writes had neither usable recents nor a registry row and briefly omitted
the running host. Registry rows are now published first and endpoint metadata
last. An early registry scan remains safe because discovery already withholds a
row whose endpoint metadata is absent; once endpoint readiness is observable,
the host is also discoverable.

Coverage in `src/workspace/catalog.rs` is provided by
`refresh_merge_preserves_a_workspace_recorded_after_its_snapshot`,
`refresh_merge_preserves_a_concurrently_changed_existing_name`,
`recents_writers_are_serialized_between_processes`,
`recents_lock_secures_a_preexisting_broad_lock_file`, and
`refresh_rejects_invalid_recents_without_rewriting_them`. The cache fallback
is covered by `unusable_optional_recents_are_omitted_from_catalog_refresh` in
the same file, while `usable_optional_recents_resolve_inside_the_cache_root`
keeps the ordinary storage path exact. Runtime-registry behavior is covered by
`an_unusable_cache_registry_falls_back_to_the_runtime_registry` in
`tests/persistent_host.rs`.

Known limitation: `flock` is advisory, so serialization depends on every
writer using the catalog update path; an older or external writer that ignores
the lock can still race with it.

## Report

The per-user workspace recents file is shared by concurrently running Runyte
processes, but catalog refresh wrote back the `recent_order` snapshot captured
before it inspected running hosts. Host inspection could take long enough for
another process to record a workspace. The first process then replaced the
file with its stale snapshot and removed the newly recorded row.

Recents updates needed to be serialized with an interprocess lock. A writer
needed to re-read and merge the current file while holding that lock
immediately before the atomic replacement, so a refresh could not overwrite
entries recorded by another process during inspection.

Relevant code was `src/workspace/catalog.rs`, `refresh`,
`record_recent_workspace_in`, and `write_recents`.
