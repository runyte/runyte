---
title: "Workspace listings disagree because new workspaces have no stored default name"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: 699e817
---

## Resolution

Commit `699e817` (`Assign default workspace names`) fixes workspace naming in
`workspace::catalog`. `WorkspaceRow::display_name` previously synthesized the
workspace directory's final component for the editor picker, while the CLI
printed the absent stored name as `-`. The two listings therefore described
the same unnamed workspace differently.

`record_recent_workspace_name_in` now assigns the directory-derived name under
the existing cross-process recents lock. `unique_default_workspace_name`
preserves the unsuffixed form for the first owner and chooses the first free
numeric suffix beginning with `-2`; names are sanitized and UTF-8-truncated to
the host-name boundary. Refresh also assigns names to legacy unnamed history
rows and supplies catalog names to running hosts which have not been explicitly
renamed. Explicit names remain authoritative.

`LocalEndpoint::store_name_if_absent` persists a newly allocated default in the
host's existing name store without replacing a name chosen by the user. CLI
lifecycle resolution now consults the workspace catalog after the running-host
registry, so a default name printed by `--wls` can also select that workspace
for stop, restart, or rename operations.

The behavior is covered by
`new_recents_receive_unique_directory_names_and_keep_them_when_revisited`,
`recent_names_fill_unnamed_running_rows_without_overriding_explicit_names`, and
`default_names_are_valid_bounded_host_names` in `src/workspace/catalog.rs`, and
by `a_new_workspace_is_listed_and_resolved_by_its_default_directory_name` in
`tests/persistent_host.rs`.

## Report

The editor's `:wls` overlay displayed an unnamed workspace using the final
component of its directory, while `runyte --wls` printed `-` in the `NAME`
column for the same workspace. For example, `/home/user/code/runyte` appeared
as `runyte` only in the overlay.

New workspaces should receive that directory-derived name as their default. If
another workspace already owns it, Runyte should append numeric suffixes in
order: `runyte`, `runyte-2`, `runyte-3`, and so on.
