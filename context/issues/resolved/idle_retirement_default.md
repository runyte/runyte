---
title: "Persistent session hosts retire after a day idle by default"
status: resolved
reported: 2026-09-25
resolved: 2026-09-25
commit: 65dd97d
---

## Resolution

Commit `65dd97d` (`Keep persistent hosts running by default`) changed
`WorkspaceConfig::default` from `1440` to `0` minutes. The host already treats
zero as disabling idle retirement and reads the setting when considering
retirement; no lifecycle or validation change was needed. Explicit positive
intervals still retire eligible clean, unattached hosts. The example config
and user guide now explain that idle persistent hosts and their language
servers continue running until explicitly stopped or a positive interval is
configured.

`src/config.rs` tests the default, and the
`WorkspaceIdleRetirementMinutes` test in `src/settings.rs` covers the live
setting's values, including a positive interval followed by zero. Both
focused tests and formatting passed. Existing host eligibility tests use
explicit intervals and do not depend on the former default.

## Report

`workspace.idle_retirement_minutes` controls how long a clean persistent
session host with no attached client, no outstanding `--wait` request and no
live terminal child keeps running before it retires. The default is `1440`
(24 hours). `0` disables retirement, so the host keeps running until it is
stopped explicitly.

Expected: the default is `0`. A persistent session stays available until it
is stopped with `:session-stop` or its host is otherwise shut down. Users who
want idle hosts cleaned up opt in by setting a positive number of minutes.

Constraints:

- Only the default changes. The meaning of `0`, the accepted range
  (`0`–`43200`) and validation stay as they are. An explicit value in a
  config file is honored as before.
- The setting keeps applying at once when changed from the `[config]`
  settings buffer.
- With retirement off by default, idle hosts for every project visited in
  persistent mode keep running, together with any language servers they own,
  until stopped. The user guide should say so where it describes retirement,
  and point to `:session-stop` and the setting as the ways to end them.

## Places that state the default

- `src/config.rs`: `Config::default` (`idle_retirement_minutes: 1440`) and
  the test asserting `Config::default().workspace.idle_retirement_minutes`
  is `1440`.
- `docs/user-guide.md`: "`workspace.idle_retirement_minutes` (1440 by
  default); zero disables retirement", and the settings-menu section that
  mentions the setting.
- `config.example.yaml`: `idle_retirement_minutes: 1440` and its comment.
- The setting's description in `src/settings.rs` ("Minutes a clean
  unattached host lives; zero keeps it") remains correct; check that it
  still reads well when `0` is the default.

## Coverage

Update the default assertion in `src/config.rs`. Tests that exercise
retirement must set a positive `idle_retirement_minutes` explicitly rather
than rely on the default. Check the persistent-host tests under `tests/` and
`src/workspace/host/` for any that relied on the old default without saying
so.
