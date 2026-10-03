The Unix recent-workspace history reader opens `workspaces.json` with blocking
`File::open` before enforcing its 8 MiB read limit. If that cache file is
replaced by a FIFO without a writer, reading the session catalog blocks
indefinitely. Startup reads of the recorded session number and history
read-modify-write operations use the same reader, so the hang can also prevent
startup or hold the history lock while another process waits.

Recent history is a bounded regular JSON file. Non-regular paths must fail
promptly, including a file replaced between an earlier path check and the
open. Preserve ordinary files, existing symlink behavior, missing-history
semantics, and the byte bound. This issue concerns the history contents read,
not a redesign of the existing Unix advisory-lock or private-directory policy.

Reproduction: create a mode-`0600` FIFO at a temporary injected history path
and call `read_recents` or `record_recent_workspace_name_in` against it. With
no writer the operation never completes. Run this in a bounded subprocess so
the regression test can kill and reap the blocked process.
