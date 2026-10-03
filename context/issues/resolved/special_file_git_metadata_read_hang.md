---
title: "Direct Git metadata reads can wait indefinitely on named pipes"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 6c5e5c4
---

## Resolution

Commit `6c5e5c4` (`fix(git): reject special files before metadata reads`).

The bounded worktree-facts reader, network shallow-boundary reader and merge
content-fingerprint reader now use the shared descriptor-checked regular-file
opener. A byte limit or earlier pathname metadata check did not prevent a
blocking FIFO open. These direct reads now reject special objects before
reading, while preserving ordinary files and their existing limits/errors.
This is independent of the deferred filesystem-plan confinement design.

`git_metadata_special_files_return_without_waiting_for_a_writer` in
`tests/git_special_files.rs` runs the owned helpers
`workspace_facts_special_file_child` and `network_shallow_special_file_child`
under a bounded parent. Separate pre-fix runs reached the five-second timeout;
the corrected runs passed. The fixture covers `.git`, HEAD, commondir, config
and shallow metadata without writing executable fixtures. All nine worktree
unit tests also passed. The parent reviewer checked each changed read and the
subprocess fixture's isolated storage and cleanup.

Known limitation: regular-file access may still wait on a stalled filesystem;
the change does not claim to eliminate pathname-substitution races generally.

## Report

Git metadata readers can block indefinitely when an expected text file is a
named pipe. The direct workspace Git-facts reader opens `.git`, `commondir`,
`HEAD`, and `config` through `File::open` before checking the opened file's
type. Listing remembered workspaces can therefore stall on a FIFO with no
writer. The commit-network reader similarly opens the shallow-boundary file
directly, outside the Git subprocess cancellation and timeout machinery.

Create a temporary workspace with a `.git` directory, a normal `HEAD`, and a
FIFO at `.git/config`, then call `read_workspace_git_facts`. It waits for a
writer instead of returning unavailable remote information. The network
reader has the same failure when its resolved shallow-boundary path is a
FIFO; a controlled Git fixture isolates this direct read from Git's own
metadata access.

These readers should promptly reject non-regular files while retaining their
existing size bounds and ordinary-file behavior. The regular-file opening
check must apply to the opened descriptor, so replacement between an earlier
metadata check and `open` cannot reintroduce a blocking FIFO read. The merge
disk fingerprint reader also needs this guarantee after its initial regular
file check. This does not change the separately deferred filesystem-path
confinement design.
