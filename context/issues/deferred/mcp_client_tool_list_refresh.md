Codex CLI ignores tool-list change notifications, so it misses context-bridge write tools granted during a session.

## Observed behavior

The `runyte-context` bridge advertises `edit_buffer`, `append_buffer` and the
terminal proposal tools only after discovery authenticates a workspace whose
native grant includes `buffer_edit` or `terminal_propose`. A Codex CLI session
(`codex-cli 0.155.0`) configured with the bridge lists the tools once at
startup. When a write grant is given during the session and `list_workspaces`
then reports `buffer_edit` and `terminal_propose` in a
workspace's `scopes`, the bridge sends `notifications/tools/list_changed`, but
the Codex session keeps offering only the read tools for the rest of its life.
Claude Code, given the same bridge and grant, adds the write tools when
notified and removes them again when a revocation or connection loss withdraws
the grant.

## Expected behavior

After `notifications/tools/list_changed`, a client issues a fresh `tools/list`
and offers the tools it returns.

## Diagnosis

The bridge sequence is correct and is covered by
`test_tool_list_refresh_after_discovery_grants_write_tools` and
`test_real_stdio_client_receives_list_changed_and_refreshed_write_tools` in
`bridges/runyte-context/tests/test_bridge.py`:

1. `initialize` declares `capabilities.tools.listChanged: true`.
2. `tools/list` before the grant exists returns read tools only.
3. `tools/call list_workspaces` returns its reply, then exactly one
   `notifications/tools/list_changed`. The notification is emitted after the
   bridge's state already reflects the new scopes, so a `tools/list` sent on
   receipt returns the refreshed inventory.
4. The next `tools/list` includes `edit_buffer`, `append_buffer`,
   `propose_terminal_text`, `terminal_proposal_status` and
   `cancel_terminal_proposal`.

The fault is in the client. In the Codex source at tag `rust-v0.155.0`,
`codex-rs/rmcp-client/src/logging_client_handler.rs` implements the handler
as a log line only:

```rust
async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
    info!("MCP server tool list changed");
}
```

The tool catalog in `codex-rs/codex-mcp/src/connection_manager/tool_catalog.rs`
serves cached tools per server; its only refresh path is for Codex's own Apps
server. This was established by reading the published source and the
installed binary's version, not by instrumenting a running Codex session.

## Minimal reproduction

Any MCP stdio server that changes its tools after a call shows the behavior;
no Runyte host is needed. This server starts with `before` and adds `after`
once `before` has been called:

```python
import json, sys
tools = [{"name": "before", "inputSchema": {"type": "object"}}]
def send(value):
    sys.stdout.write(json.dumps(value) + "\n"); sys.stdout.flush()
for line in sys.stdin:
    message = json.loads(line)
    method, ident = message.get("method"), message.get("id")
    if ident is None:
        continue
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": ident, "result": {"protocolVersion": "2025-06-18",
              "capabilities": {"tools": {"listChanged": True}},
              "serverInfo": {"name": "refresh-probe", "version": "0"}}})
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": ident, "result": {"tools": tools}})
    elif method == "tools/call":
        send({"jsonrpc": "2.0", "id": ident,
              "result": {"content": [{"type": "text", "text": "ok"}]}})
        if len(tools) == 1:
            tools.append({"name": "after", "inputSchema": {"type": "object"}})
            send({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"})
    else:
        send({"jsonrpc": "2.0", "id": ident, "result": {}})
```

Register it as a Codex stdio MCP server, ask the session to call `before`, then
ask it to call `after`. The client never offers `after`, although the server
announced it and returns it from any later `tools/list`.

## Constraints

A bridge-side fix must not weaken native authorization or advertise mutation
tools that the current grants cannot use.

The bridge now runs discovery once before its first `tools/list` reply and keeps
only the granted scopes (`Bridge.prime_scopes` in
`bridges/runyte-context/runyte_context/client.py`). A Codex session therefore
sees write tools whose grant existed when it started. Grants made during the
session still need a new Codex session, and a revocation during the session
leaves the tools listed until calls fail. Always advertising the write tools
was rejected: it advertises unusable mutations and conflicts with the
agent-context plan. Resolving the rest requires Codex to re-fetch tools on
`notifications/tools/list_changed`.
