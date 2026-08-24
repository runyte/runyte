---
title: "An edited file created through the explorer sometimes cannot be saved"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: e322792
---

## Resolution

Commit e322792 (`Keep explorer-renamed files savable`) fixes exact-path
retargeting in `mapped_applied_path`. When a confirmed explorer plan renamed
or moved an already-open file, the function represented the exact match as an
empty suffix and evaluated `to.join("")`. Rust retained the resulting trailing
separator in the path's operating-system string even though `Path` equality
treated it as equal to the separator-free destination. The open buffer
therefore appeared correctly retargeted to existing tests but later asked the
operating system to inspect a regular file through a directory-shaped path.

Exact matches now take the confirmed destination directly; only descendants
of a moved directory are joined to a suffix. `Buffer::retarget_path` also
refreshes the recorded disk state when the destination has the same stable
identity, contents, modification time, and access metadata as the source.
This accounts for the ctime change caused by a Unix rename without accepting
an unrelated replacement or a concurrent change as the file the buffer
originally read.

Coverage is provided by
`app::tests::filesystem_rename_reopens_the_same_language_at_a_savable_new_path`
in `src/app.rs`, which checks the exact path spelling, LSP retargeting, and a
successful guarded save after the rename, and
`buffer::tests::retargeting_does_not_accept_an_unrelated_destination_state` in
`src/buffer.rs`, which proves a retarget race still preserves the newer
destination and refuses the save.

## Report

After creating a new file in the explorer, opening it, and typing text, saving
sometimes failed. Different errors were observed on macOS and Linux but were
not retained. The most recent Linux failure was:

```text
ERROR · 2026-08-20 23:27:29 · Runyte · Action failed
failed to inspect /home/user/project/example.md/
```
