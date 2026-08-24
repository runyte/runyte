---
title: "Workspace mode is missing from editor settings"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: e990183
---

## Resolution

Commit e990183 (`Expose workspace mode in editor settings`) resolved the issue.

`WorkspaceConfig::mode` was already loaded from YAML, but `SettingId` and the
registry that generates the `[config]` buffer had no workspace-mode entry.
Consequently, the configuration page could neither display the saved value nor
offer the two values accepted by configuration loading.

The registry now contains a typed `workspace.mode` setting with finite
`standalone` and `persistent` choices, ordered with the workspace configuration
immediately after the editor settings. Its value is patched losslessly into the
loaded YAML and the generated page refreshes to show the saved choice.

Workspace mode uses the restart-required policy deliberately. Automatic
persistent-mode selection happens before `App` is constructed, so changing the
setting cannot convert the process that owns the current editor session. The
picker therefore keeps the current process policy effective and reports that
the saved value applies after restarting Runyte, specifically to future bare
launches.

Tests covering the behavior are
`settings::tests::registry_has_stable_unique_keys_ids_and_typed_configured_values`
and `settings::tests::workspace_mode_persists_as_a_typed_unquoted_yaml_choice`
in `src/settings.rs`, plus
`app::tests::workspace_mode_is_visible_and_saved_for_future_launches_only` in
`src/app.rs`.

Known limitation: persistent workspace hosts remain unsupported off Unix. The
setting stays visible so the configuration remains discoverable and portable,
but a persistent launch is rejected on an unsupported platform.

## Report

The Runyte workspace mode setting, `standalone` versus `persistent`, did not
appear in the configuration view opened with `Space o o`. The setting was
accepted by configuration loading, but it was not visible in the editor's
generated configuration page, so the available mode and its current value
could not be discovered or reviewed there.

The `Space o o` configuration view was expected to include the workspace mode
setting and show whether Runyte was configured for `standalone` or `persistent`
operation.

Relevant areas were workspace configuration in `src/config.rs` and the
generated configuration buffer opened by `Space o o`.
