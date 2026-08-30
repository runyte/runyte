---
title: "A long-running client cannot start another workspace after its executable is replaced"
status: resolved
reported: 2026-08-30
resolved: 2026-08-30
commit: d870473
---

## Resolution

Commit d870473 (`Harden cross-platform integration test boundaries`) added
`UnavailableStartupExecutable` at the shared detached-host boundary in
`src/workspace/lifecycle.rs`. `start_detached_host` now classifies a spawn
`NotFound` only when the configured path-like executable independently probes
as missing. Existing executables, bare names resolved through `PATH`, missing
interpreters, and other metadata failures keep the ordinary spawn diagnosis.
No fallback binary is searched for or launched.

`start_workspace_switch_host` in `src/main.rs` adds the switch-specific advice
to detach with `:detach`, launch Runyte again, and retry. The typed failure is
then handled through the ordinary prepared-switch transition, which publishes
the notice without replacing the current source endpoint or manufacturing a
previous endpoint.

Coverage is provided by
`missing_startup_executable_is_diagnosed_as_a_replaced_client` and
`an_existing_startup_target_keeps_the_generic_spawn_diagnosis` in
`tests/persistent_host.rs`; and
`tests::replaced_executable_switch_failure_explains_how_to_recover` in
`src/main.rs`.

## Report

### Observed behavior

A persistent Runyte client can remain attached while the executable from which
it was launched is rebuilt, upgraded, moved, or removed. The running process
continues to work, but an operation that needs to start another persistent
workspace host can then fail with a misleading error:

```text
cannot start destination workspace host for /home/me/code/project-feature: No such file or directory (os error 2)
```

This was observed after `Space g w`, `Tab N` successfully created a branch and
linked worktree. Persistent mode then attempted the documented automatic
attachment to the new worktree and returned to the source workspace with the
error above. Selecting the created worktree from `Space g w` and pressing
`Enter` produced the same failure. Repeated attempts are coalesced by the
notification center.

The destination is not the missing path in this case. `prepare_switch_target`
initializes the destination and `start_detached_host` canonicalizes its working
directory before constructing and spawning the child command. The observed
message is added only when `Command::spawn` fails.

`prepare_switch_target` supplies `std::env::current_exe()` as the executable for
the destination host. On Unix, replacing or unlinking the executable of a
running process can leave `current_exe` naming a path that no longer exists;
Linux commonly exposes such a path with a ` (deleted)` suffix. Spawning that
path returns `NotFound`, but the existing error context names only the
destination workspace. It therefore suggests that Git failed to create the
worktree even though creation completed successfully.

The same underlying condition can affect other lifecycle operations that use
the current executable to start a detached host, including attach, restart,
and `--wait` paths.

### Expected behavior

When detached-host startup returns `NotFound` and the configured startup
executable itself is no longer available, Runyte should explain that the
client's executable was probably rebuilt, moved, or upgraded while the client
was running. For a failed workspace switch, the message should tell the person
to run `:detach` and launch Runyte again. The source persistent session must
remain running and the client must continue returning to it as it does for
other unreachable switch destinations.

If the executable still exists, or checking it fails for another reason,
Runyte should preserve the ordinary spawn error. `NotFound` can also describe
a missing executable interpreter or a race involving another spawn resource,
so the message must not claim an executable replacement without evidence.

Detection should be based on the executable's availability rather than
matching a platform-specific ` (deleted)` string. It should happen at the
shared detached-host lifecycle boundary so every affected caller receives the
same diagnosis, while allowing callers to add context-appropriate recovery
instructions.

Runyte should not silently search `PATH` or launch a different on-disk binary.
Doing so could cross a protocol or configuration version boundary without the
person explicitly restarting the client.

### Reproduction

1. Build Runyte and launch it in persistent mode from that build.
2. While the interactive client remains running, rebuild or replace the
   executable it was launched from.
3. Open `Space g w` and use `Tab N` to create a branch and linked worktree, or
   press `Enter` on a worktree with no running persistent host.
4. Observe that the worktree exists, the client returns to the source
   workspace, and host startup reports `No such file or directory` as though
   the destination were missing.
5. Detach and launch the new Runyte executable for the destination; attachment
   succeeds without recreating the worktree.

### Regression coverage

Lifecycle coverage should pass a deliberately missing executable path to
detached-host startup and assert the actionable replacement/restart diagnosis.
It should also retain the existing generic spawn diagnosis when the executable
path exists. Workspace-switch coverage should confirm that this failure is
reported after the client safely reattaches to its source workspace.

The test does not need to rebuild, overwrite, or execute a file it created.
The existing `creating_a_worktree_starts_and_attaches_its_persistent_session`
integration test covers the successful path with a stable bundled executable,
but not replacement of that executable while its client remains alive.
