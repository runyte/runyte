---
title: "Workspace modes used inconsistent host and client terminology"
status: resolved
reported: 2026-08-18
resolved: 2026-08-18
legacy_commit: 09bf13d
---

## Resolution

Commit 09bf13d (`Unify workspace command-line terminology`) made
**standalone** and **persistent** the user-facing values on the workspace-mode
axis. `SessionRole::Client` had caused a persistent session to label itself
`client` in the status line; the presentation type is now `SessionMode`, and
the transported-frame path labels that mode `persistent`. The workspace
management `LaunchMode` variants and their selector/name fields now use
workspace terminology as well.

The command line now selects persistent mode with `--persistent` or `-a`.
`--attach` was removed rather than retained as another compatibility spelling.
Help defines a workspace and the two modes before listing their flags, places
`--serve` and `--wait` with persistent workspace operations, states the Unix
boundary, and no longer exposes the process-only `--project-root` option.
`--project-root` continues to parse because detached startup still uses it
internally. `README.md` now introduces the same model before using host/client
only where the process architecture requires that distinction.

`App::execute_colon_invocation_for_workspace_platform` had gated the entire
workspace picker on `persistent_workspace_switch`, even though its workspace
service reads the same registry and visited-history catalog used by
`runyte --wls`. The listing now opens in standalone mode. Activation checks
the mode only when Enter attempts a switch, while workspace start and stop
enforce the same boundary in their operation methods so the picker action and
colon-command paths cannot disagree. All three refusals name the required
`workspace.mode: persistent` setting. History-only catalog rows now leave
host-owned values as `None`; both the terminal table and picker omit those
unavailable values instead of printing question marks or invented zero/false
answers.

Coverage lives in
`app::tests::standalone_can_list_workspaces_but_cannot_switch_or_manage_them`
in `src/app.rs`,
`ui::tests::the_status_row_names_the_workspace_mode_before_the_workspace` in
`src/ui.rs`,
`workspace::catalog::tests::stopped_workspace_id_matches_the_running_endpoint_identity`
in `src/workspace/catalog.rs`,
`launch::tests::workspace_modes_are_explicit_and_mutually_exclusive` in
`src/launch.rs`,
`editor_help_hides_internal_options_and_uses_workspace_modes` in
`tests/release_packaging.rs`, and
`hosts_list_name_restart_and_resolve_by_id_name_or_directory` in
`tests/persistent_host.rs`.

## Report

Runyte used *workspace*, *host*, *client*, *standalone mode*, and *persistent
mode* without explaining how they related, and its surfaces did not agree.

The configuration offered `workspace.mode` with the values `standalone` and
`persistent`. The status line showed `standalone` in one mode and `client` in
the other. Those were not two values of one thing: `standalone` was a mode and
`client` was a role, so setting `workspace.mode: persistent` produced a status
line that answered a question that was never asked. The expected label was
`persistent`.

`runyte --help` had the same split. `--standalone`, `--serve`,
`--project-root`, `--attach`, and `--wait` sat under `OPTIONS:` while the
`--workspace-*` family sat under `WORKSPACES:`, and none of the first group
used either of the two words used by the configuration. Nothing in `--help`
said that the `WORKSPACES:` section described something that existed only in
one of the two modes.

`:wls` in standalone mode failed with:

```text
ERROR · 2026-08-18 08:51:53 · Runyte · Action failed
workspace listing is available only in persistent mode
```

That described the command rather than the capability, since `runyte --wls`
could list workspaces from a standalone shell.

The user-facing vocabulary needed to be two words on one axis:
**standalone** and **persistent**. *Host* and *client* belonged only in
`README.md` where the architecture genuinely needed to distinguish the
process that owns the state from the terminal attached to it, not on the
status line or in `--help`. Internal names carried the older vocabulary too:
`SessionRole` in `src/ui.rs` and `LaunchMode::ListHosts`, `ShutdownHost`,
`RestartHost`, `NameHost`, and `RenameHost` in `src/launch.rs`.

On the command line, flags needed to read as choices of mode. `-a` and
`--attach` were how a user requested persistent mode from a shell configured
for standalone, and needed to be spelled as persistent mode. `--serve` and
`--project-root` were not modes; they were how persistent mode started and
addressed its own process. `--project-root` was passed by Runyte to the process
it spawned and, like `--cwd-file`, needed to disappear from `--help` while
remaining a working option. Everything mode-related needed one section with a
short preamble naming the two modes before any flag was listed.

`:wls` needed to work in standalone mode and show the same inventory as
`runyte --wls`, because reading the visited-history cache and runtime registry
needed no host. Columns only a host could answer, such as the unsaved-buffer
count, were to be blank there. Standalone mode could not switch: Enter on a
row needed to explain that switching workspaces required
`workspace.mode: persistent` rather than refusing to open the listing.
`:wat` and `:wst` were the same case and needed the same style of explanation.

`README.md` and `--help` lacked an introduction explaining what a workspace
was, what the two modes did differently, which flags belonged to each, and
that persistent mode was currently Unix-only. The README section assumed the
reader already knew that model, but nothing before it supplied the model.
