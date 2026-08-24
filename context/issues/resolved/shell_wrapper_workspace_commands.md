---
title: "The documented shell wrapper prevents workspace commands from running"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: 6aee46f
---

## Resolution

Commit `6aee46f` (`Allow shell wrapper for workspace commands`) fixes the
launch validation in `main.rs`. The `run` function rejected `--cwd-file` before
executing workspace-management modes even though the documented shell function
adds that option to every invocation. Those commands have no directory handoff
to report, but rejecting the unused capability made the wrapper cease to be a
transparent way to invoke Runyte.

Workspace-management commands now accept `--cwd-file` and leave it untouched.
Standalone and attached clients retain the existing behavior of writing the
selected directory only after a successful `:quit-here`.

The behavior is covered by
`hosts_list_name_restart_and_resolve_by_id_name_or_directory` in
`tests/persistent_host.rs`, which runs both `--workspace-name` and `--wls`
through a shell handoff file and verifies that the management commands do not
write it.

## Report

On macOS, the documented shell function always supplies `--cwd-file`. Runyte
was normally used in standalone mode, with occasional `ru -a` launches for
testing. A subsequent workspace-list invocation:

```text
ru --wls
```

reported instead of listing workspaces:

```text
Error: --cwd-file is available only in standalone mode
```
