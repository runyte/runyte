---
title: "MCP acceptance discovers a workspace before its context service is ready"
status: resolved
reported: 2026-09-21
resolved: 2026-09-21
commit: fd1cd7b
---

## Resolution

Commit `fd1cd7b` (`Wait for readable workspace discovery in MCP acceptance`)
corrects the real-editor fixture's readiness assumption. `NativeEditor` waits
for the first fixture text to render, but standalone startup deliberately
draws that frame before `start_host_services` starts context access and
publishes endpoint registration. An immediate successful discovery response
may therefore contain no readable workspace. Endpoint-file existence alone
would also be insufficient: discovery requires a successful live probe.

Both real-editor scenarios now wait for readable discovery of their exact
fixture project roots. The helper polls only successful, incomplete inventories
within one 15-second budget, passes the remaining time into the RPC reader,
and propagates RPC/tool errors immediately. Timeout diagnostics retain only
bounded root, readability, unavailable-reason and truncation fields. Product
startup ordering and service deadlines are unchanged.

`bridges/runyte-context/tests/test_workspace_readiness.py` covers this contract
with a controlled clock: `test_delayed_discovery_requires_every_exact_root_and_readable_access`,
`test_missing_workspace_expires_with_bounded_selected_diagnostics`,
`test_discovery_errors_propagate_without_retry`, and
`test_late_success_cannot_extend_the_startup_deadline`. All four pass locally.
The real scenarios remain in `bridges/runyte-context/tests/test_runyte.py`:
`test_concurrent_appends_from_two_agents_land_whole_without_a_revision` and
`test_two_agent_clients_read_live_and_detached_workspaces_edit_unsaved_and_observe_revocation`.
Their Unix CI acceptance is pending at the time of this record.

## Report

The Linux plugin-conformance job in
[run 35609722577](https://github.com/runyte/runyte/actions/runs/35609722577)
failed on 2026-09-21 at `tests/test_runyte.py:397`. After opening the standalone
editor, the concurrent-append test queried `list_workspaces` once and selected
the workspace whose project root ended in `one`:

```python
workspace = next(row['workspace'] for row in rows if Path(row['root']).name == 'one')
```

That selection raised `StopIteration`. The separate live/detached-workspace
scenario passed in the same job. The fixture should wait for its required
readable workspace to become discoverable before sending buffer operations,
without treating an early valid empty inventory as a product failure or
silently retrying an actual RPC error.
