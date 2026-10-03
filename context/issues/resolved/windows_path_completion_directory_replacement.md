---
title: "Windows path completion can reuse a replaced directory listing"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 82cfd0f
---

## Resolution

Commit `82cfd0f` (`fix(completion): validate cached Windows directory identity`).

`directory_listing::directory_identity` previously returned no identity on
Windows, while a settled unchanged modification time allowed indefinite cache
reuse. It now uses the existing native `windows_fs::Identity` helper on the
canonical directory. A listing without an obtainable identity uses bounded
volatile reuse instead of claiming indefinite freshness. Unix identity behavior
and the completion ordering remain unchanged.

`a_replaced_directory_cannot_reuse_the_previous_objects_listing` in
`src/directory_listing.rs` now runs on Windows as well as Unix and replaces a
directory while preserving its timestamp. The Linux directory module suite
passes (159 tests across directory-related modules).

Known limitation: the Windows-specific branch requires native Windows CI;
this session validated the shared behavior and existing native helper contract
on Linux, not Windows execution.

## Report

Path completion can retain an obsolete directory listing indefinitely on
Windows after another directory replaces it at the same pathname. The listing
cache checks the filesystem object identity only on Unix. On Windows it stores
no identity, but still treats an old, unchanged modification time as sufficient
to reuse a listing without an expiry.

Reproduction: create two directories with different entries and give both the
same modification time older than two seconds. Complete a path inside the first
directory so its listing is cached. Rename it aside, then rename the second
directory onto the first pathname. Completion continues offering the removed
directory's entries rather than the replacement's entries.

Windows directory identity must participate in cache invalidation. If identity
cannot be obtained, the existing short reuse window must bound stale results;
an unchanged timestamp alone must not grant unlimited reuse. Ordinary unchanged
directories should continue using the cache instead of being read each time a
key or redraw requests completion.
