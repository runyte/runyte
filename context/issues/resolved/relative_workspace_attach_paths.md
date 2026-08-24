---
title: "Relative workspace attach paths resolve from the wrong directory"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: 3fcbd1b
---

## Resolution

Commit `3fcbd1b` (`Resolve workspace attach paths from editor cwd`) fixes the
workspace-switch handoff. `App::request_workspace_switch` previously retained
only the selector, so the attached client interpreted a relative path from its
own process current directory after the editor had detached. That disagreed
with the directory owned by `App`, particularly after `:cd`.

The switch request now captures `App::working_directory` alongside the
untouched selector. The bundled local protocol deliberately moves to version
16 to carry both values across the host/client boundary. Running-host and
recent-workspace resolution use the captured directory only when interpreting
the selector as a path; exact workspace names, full IDs, and unambiguous ID
prefixes continue to compare against the original selector.

Tests covering the behavior are:

- `workspace_attach_captures_the_editor_working_directory_for_relative_selectors`
  in `src/app.rs`.
- `known_selector_paths_use_the_supplied_editor_directory_and_ids_and_names_stay_exact`
  in `src/workspace/catalog.rs`.
- `relative_workspace_attach_uses_editor_cwd_and_keeps_one_client_process` in
  `tests/local_protocol.rs`.
- `protocol_version_and_request_bounds_are_explicit` in `src/protocol/mod.rs`.

## Report

Priority: P2.

The `:workspace-attach PATH` command stored a relative `PATH` unchanged. This
was especially visible after `:cd`: path completion was generated relative to
the editor's `App::working_directory`, but the attached client later resolved
the stored selector against its own process current directory.

For example, `:workspace-attach ../project` could switch to a different
directory from the one shown by completion. Relative path selectors needed to
be resolved against the editor working directory before the workspace switch
was handed off. ID and name selectors needed to retain their existing selector
semantics rather than being unconditionally converted into paths.

Relevant code was `src/app.rs`, `ColonCommand::WorkspaceAttach`, and the
workspace switch handoff.
