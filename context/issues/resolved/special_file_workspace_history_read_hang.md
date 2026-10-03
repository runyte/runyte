---
title: "Recent-workspace history reads can block indefinitely on named pipes"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: d1eb280
---

## Resolution

Commit `d1eb280` (`fix(session): reject special files in recent workspace history`)
uses the descriptor-checked regular-file opener in the Unix history reader.
Its byte ceiling previously applied only after a blocking open had completed,
so a FIFO could stop startup/catalog reads or a history update while holding
its lock. The reader now rejects special objects promptly; the existing byte
limit, absent-history result, symlink behavior and lock policy are preserved.

`special_file_history_reads_refuse_without_blocking` and its owned helper
`special_file_history_fixture` in `src/workspace/recent_history/tests.rs` cover
FIFO and alias refusal, read-modify-write preservation, missing history, and
ordinary alias recovery. The regression reached its ten-second deadline
before the fix. Afterward all nine history tests passed; the helper is
intentionally ignored outside its bounded parent. The parent reviewer checked
the changed reader and fixture independently.

Known limitation: regular-file access can still wait on a stalled filesystem.

## Report

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
