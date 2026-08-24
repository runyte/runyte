---
title: "Space r does not reload files or refresh explorers"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: 3908d2c
---

## Resolution

Commit 3908d2c (`Add Space reload command`) added `Space r` to the shared
keymap as a route to the existing typed `reload` colon identity. The
`reload_active` dispatch in `src/app.rs` now chooses behavior from the active
buffer: file buffers use the existing disk reload, while directory buffers
use `refresh_directory`, preserving its confirmation before dirty explorer
edits are discarded. Because Vim delegates the Space application tree to the
same registry, the binding works in both editing grammars without duplicate
grammar logic.

Coverage is in `space_r_reuses_the_reload_command_identity` in
`tests/keymap.rs` and `space_r_reloads_files_and_refreshes_directories` in
`src/app.rs`. The historical removed-binding audit in `tests/keymap.rs` no
longer lists `Space r` as intentionally unbound.

## Report

A `Space r` binding was requested, working in both files and the explorer:
reloading the file, or refreshing the directory listing in the explorer.
