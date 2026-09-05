---
title: "A workspace host of an older protocol is reported as stopped and cannot be reached or ended"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: 69fcf78
---

## Resolution

Fixed in `69fcf78` "Reach a workspace host of another protocol".

Two independent doors led to the same workspace and disagreed about it.
`--list-workspaces` goes through `registered_hosts_in`, which reads the host
registry; `validate_registered_metadata` discards any entry whose `id` is not
32 hex characters, and metadata written before registered identities existed
carries no `id` at all. With no registry entry, the workspace fell through to
the recents list and was printed as `stopped`. Every client that actually
opens the workspace resolves its endpoint from the project root instead, found
the old host there, and refused.

The refusal itself was ordered wrongly. `verify_compatible_for_connect`
compared protocols before anything examined the recorded process, so a host
that had already exited was refused on its protocol forever: no code below
that check ever ran to notice the process was gone, and the socket and
metadata it left behind could never be cleared or replaced. The endpoint stayed
poisoned until someone deleted the files by hand.

`verify_compatible_for_connect` now splits on liveness. A recorded process
that is still running yields the new `IncompatibleHost` error, which carries
the protocol, the PID, and the project root, and whose message names the
command that ends it. A process that has exited yields an `io::NotFound`
error, which `is_stale_endpoint_error` already classifies as stale, so the
recovery every caller performs — `LocalServer::bind` unlinking a socket whose
recorded host is dead — clears it like any other leftover. `--wait` and
`ensure_workspace_host` decide on that type rather than on the word
"incompatible" appearing in a message, which is what they matched before.

`LocalEndpoint::published_host` reads what an endpoint publishes without
requiring a protocol this build can speak, verifying the project root, the
socket identity, the recorded process, and that the socket still accepts a
connection. `catalog::refresh` calls it for every recent project the registry
did not account for, so an unregistered live host is described rather than
reported as stopped: an incompatible one as `running (protocol N)` with
unknown buffer counts, a compatible one inspected over a control connection as
usual. `WorkspaceRow::state_label` is the single wording, shared by the CLI
table and the editor's workspace picker.

`lifecycle::terminate_incompatible_host` ends such a host and then clears its
endpoint through `LocalEndpoint::clear_published_host`, which refuses to remove
an endpoint another process has since published. `--shutdown-workspace` asks
first and only falls back to termination once `IncompatibleHost` proves that
asking is impossible, printing that it stopped the process without asking and
that unsaved buffers are lost. Terminating a host that speaks the current
protocol is refused, so the abrupt path cannot stand in for the protocol one:
only the host itself can refuse a shutdown over its unsaved buffers.

The catalog's discovery needed the configured workspace state directory to
resolve a project's endpoint, so `known_workspaces`, `resolve_known_workspace`,
and `resolve_known_workspace_from_directory` take it, and `refresh` takes an
optional runtime-directory override so tests never read or write the real one.

Tests:

- `an_incompatible_endpoint_is_stale_once_its_host_has_exited` in
  `src/workspace/transport.rs` covers the ordering rule in both directions.
- `a_live_incompatible_host_is_listed_and_can_be_stopped` in
  `tests/local_protocol.rs` publishes an older endpoint over a live process and
  drives the real binary through listing, stopping, and listing again.
- `wait_preserves_the_error_from_a_live_incompatible_host` in
  `tests/local_protocol.rs` now also asserts that the error names the process
  and `--shutdown-workspace`.

Known limitation: discovery of an unregistered host is driven by the per-user
recents list, so a running host whose project was never recorded there is still
absent from `--list-workspaces`. A directory selector reaches it regardless, as
`runyte --shutdown-workspace <directory>` resolves the endpoint from the
project rather than from the registry.

## Report

`git merge` failed in `/home/user/code/runyte` with the editor configured as
`runyte --wait`:

```
user@host:~/code/runyte$ ru -l
ID                                NAME  DIRECTORY                           STATE    DIRTY  TUI
--------------------------------  ----  ----------------------------------  -------  -----  ---
658471a65ca7c48244bef5867d3e80bc  -     /home/user/code/runyte              stopped  0      no
3ab49e5b3d4fc8cc6600e204dd5a571e  -     /home/user/code/runyte-dev          stopped  0      no
bd9c7816e7aff88b40932e999f8c2337  -     /home/user/code/runyte-host-client  stopped  0      no
user@host:~/code/runyte$ git merge editor-fixes
Already up to date.
user@host:~/code/runyte$ git merge editor-fixes
hint: Waiting for your editor to close the file... Error: workspace host protocol 9 is incompatible with client protocol 18; restart the host
error: there was a problem with the editor 'runyte --wait'
Not committing merge; use 'git commit' to complete the merge.
```

Every workspace was listed as stopped, including the one the error came from.
The problem had occurred before.

The host behind it was a detached process from an earlier build, still running
against protocol 9 with its executable already replaced on disk, holding
`$XDG_RUNTIME_DIR/runyte/658471a65ca7c48244bef5867d3e80bc/`. It was absent from
both host registries. The error's advice to restart the host named no way to do
so: `--list-workspaces` did not show it, and `--shutdown-workspace` failed with
the same protocol error, leaving `kill` as the only remaining exit.
