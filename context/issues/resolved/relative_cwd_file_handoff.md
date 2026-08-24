---
title: "Relative cwd handoff paths changed identity across workspace switches"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: bea1709
---

## Resolution

Commit `bea1709` (`Preserve relative cwd handoff across workspace switches`)
fixed launch-time handling of `LaunchArguments::cwd_file` in `src/main.rs`.
`run` had retained a relative handoff path until after the editor requested a
workspace switch. `launch_workspace_process` then started the replacement from
the destination project root and forwarded that still-relative path, so the
replacement resolved it to a different file from the one the invoking shell
was waiting on.

Launch now resolves a relative `--cwd-file` against the original process
directory before project discovery can start or attach to a host. The absolute
path is carried unchanged by the attached-client switch loop, so repeated
persistent workspace switches retain one handoff-file identity. Standalone
mode no longer switches workspaces. An absolute path supplied by the caller
remains unchanged.

Covered by
`tests::relative_cwd_file_keeps_the_invoking_shells_identity_after_directory_changes`
and `tests::absolute_cwd_file_is_forwarded_unchanged` in `src/main.rs`. The
first test reuses the path after changing directories, writes the handoff, and
verifies that the original caller-side file exists while the destination's
same-spelling file does not.

## Report

When `--cwd-file` was supplied as a relative path and an editor switched
workspaces, the path was forwarded to the replacement process after that
process's current directory was changed to the destination project root. The
replacement consequently wrote a different file from the one the original
shell was waiting on.

The directory-handoff file path needed to be resolved to an absolute path
before changing directories or spawning the replacement process. The relevant
code was the workspace-switch replacement startup and `--cwd-file` forwarding
in `src/main.rs`.
