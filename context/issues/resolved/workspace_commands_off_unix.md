---
title: "Workspace commands appear available on unsupported platforms"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: c3b89b3
---

## Resolution

Commit `c3b89b3` (`Mark workspace commands unavailable off Unix`) corrected the
platform capability and execution boundaries for persistent workspace
commands. `AppCapabilitySnapshot::command_availability` previously fell
through to `CommandAvailability::Available` for all four commands, even where
the platform could not host a persistent workspace. Their execution paths
then discovered the limitation too late, and `workspace-attach` checked the
persistent-mode flag before it checked platform support.

The capability snapshot now owns the persistent-workspace-host availability,
computed from the target platform through a testable helper. Workspace attach,
list, start, and stop all read that capability while remaining in the shared
command inventory. Their execution paths use the same platform gate and reason
before applying the existing Unix persistent-mode checks, so unsupported
platforms cannot produce the misleading mode-disabled error.

Coverage is provided by
`workspace_commands_stay_in_the_palette_and_share_platform_availability` and
`workspace_execution_reports_the_shared_unsupported_platform_reason_first` in
`src/app.rs`, plus
`workspace_host_platform_capability_uses_the_policy_reason` and the expanded
`one_capability_snapshot_drives_syntax_and_lsp_commands` in
`src/service_health.rs`.

Known limitation: the unsupported path is exercised through the injected
platform boundary on the Unix development host. A Windows-target `cargo check`
could not reach Runyte because the environment lacks the
`x86_64-w64-mingw32-gcc` cross compiler required by tree-sitter dependency
build scripts.

## Report

Priority: P2.

On non-Unix platforms, the four workspace commands remained available in the
command palette and reported an error only after execution.
`workspace-attach` could additionally report the misleading error that
persistent mode was disabled rather than explaining that persistent workspace
hosts were unsupported on the platform.

The platform policy in `context/issues/windows_support.md` requires these
commands to remain in the shared command inventory but return
`CommandAvailability::Unavailable` off Unix, with the correct unsupported
platform reason. Their palette rows must be dimmed consistently with other
unavailable commands.

Relevant code: `src/app.rs`, command capability calculation and execution of
the workspace attach, list, start, and stop commands.
