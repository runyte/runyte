---
title: "Malformed workspace recents were erased during catalog refresh"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: 445ab2d
---

## Resolution

Commit `445ab2d` (`Preserve malformed workspace recents`) fixed
`workspace::catalog::refresh`, which converted every error from `read_recents`
into an empty list with `unwrap_or_default`. Refresh then continued through
host inspection and wrote that empty list back to `workspaces.json`, replacing
the unreadable catalog instead of reporting that it could not be decoded.

Refresh now propagates the recents read or JSON decode error before host
inspection and writeback. A malformed, truncated, or incompatible recents file
is therefore reported to the listing caller and its original bytes remain
untouched.

Covered by
`workspace::catalog::tests::refresh_rejects_invalid_recents_without_rewriting_them`
in `src/workspace/catalog.rs`, which checks truncated JSON, malformed JSON, and
an incompatible JSON shape and verifies byte-for-byte file preservation for
each case.

## Report

Workspace catalog refresh read `workspaces.json` with `unwrap_or_default()`.
If the file was truncated, malformed, or used an incompatible format, refresh
silently treated the catalog as empty and later rewrote the same file as `[]`.

Merely listing workspaces could therefore destroy all stopped-workspace
history. A recents read or decode failure needed to be propagated to the
caller, and refresh needed to leave the original file intact instead of
replacing it with an empty catalog.

The relevant path was `refresh`, `read_recents`, and the writeback performed
after host inspection in `src/workspace/catalog.rs`.
