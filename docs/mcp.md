# MCP access for agents

Runyte includes a local MCP stdio server. No Python, package installation, API
key, or TCP listener is needed. Each agent launches its own server process:

```sh
/path/to/runyte mcp --identity agent
```

In a generic MCP client configuration:

```json
{"mcpServers":{"runyte":{"command":"/path/to/runyte","args":["mcp","--identity","agent"]}}}
```

For a client using TOML MCP configuration:

```toml
[mcp_servers.runyte]
command = "/path/to/runyte"
args = ["mcp", "--identity", "agent"]
```

Use the absolute path to the binary you want to test. Windows uses the same
arguments with an absolute path to `runyte.exe`. `runyte mcp --help` describes
the options. `--timeout 2` sets the per-host deadline (0.1–10 seconds).

## Permissions

Open `:mcp agent` in each workspace to share. The identity must match
`--identity`; its label does not authenticate an agent executable. Use separate
identities, such as `codex` and `claude`, when permissions should differ.

Toggle reads, buffer edits, and terminal proposals with `1`–`4`, optionally
remember the grant with `r`, then Tab and Enter to **Apply permissions**.
Write permissions automatically enable the corresponding read permission.
`:mcp` reopens the current grant, lifetime and connected-reader count; `n`
cycles known identities, and `x` revokes the selected identity. Readers connect
when the agent performs discovery; tool listing alone leaves the count at zero.
Revocation drops
readers and pending proposals. `:context-access` remains an alias.

**Starting the agent before granting is fine.** The nine tools stay available;
a tool's presence is not permission to use it. Calls check the target's native
grant. After a permission change, run `find_resources` again. Healthy searches
preserve handles. Changed permissions renew the connection and require fresh
handles.

The default identity is `agent`. Without Remember, a grant lasts until the
owning editor or persistent host exits. Private credentials and remembered
grants stay in Runyte's account cache, outside the project. `RUNYTE_CONTEXT_HOME`
can select an absolute private store shared by the editor and MCP process.
See the [user guide](user-guide.md#what-a-grant-covers) for platform locations.

## Finding the right target

Ask the agent to search contents as well as titles:

> Find the terminal showing Claude in this project and read its latest output.

The usual sequence is two calls:

1. `find_resources(workspace="/projects/example", query="claude", kind="terminal")`
2. `read_terminal(terminal=<returned handle>)` or
   `propose_terminal_text(terminal=<returned handle>, text="...")`

`find_resources` uses the editor's fuzzy matcher over names and content.
`clde` can match `Claude Code` even when the terminal is named `shell`.
Results include match excerpts, the workspace, exact handles and buffer
revisions. Several matches remain several candidates: the server never picks
a write target by fuzzy similarity. A query is optional; omitting it lists
resources. `match_in="name"` avoids content reads; `match_in="content"`
restricts matching to text. Matching uses smart case, as the editor does.

Search is intentionally bounded: four workspaces per page, eight buffers and
eight terminals per workspace by default (up to 32 each), the first 8,192
characters of each buffer, and up to 80 recent terminal rows. Follow
`next_workspace_offset` and the per-workspace/per-kind `next_offset`; use
`content_from` for a later buffer window. A three-second content-search budget
returns partial progress. Discovery and authentication have their own bounded
deadlines. Empty results do not prove that text is absent. No filesystem scan
runs, and source text is not cached between calls.

Workspace selectors accept a project path fragment or returned workspace
handle. Omitting one searches the discovered authorized workspaces; it never
uses current editor focus. `include_hidden=true` explicitly broadens discovery
beyond the current environment. `list_workspaces` checks identity and grants
without reading source content.

## Reading and writing

- `read_buffer` accepts just a discovered buffer handle for its first 16,384
  characters at the discovered revision. Optional `from`, `to`, and
  `expected_revision` give exact Unicode-character ranges. Stale reads require
  rediscovery. `read_terminal` defaults to 80 recent rows; `region="screen"`
  reads the live screen.
- `append_buffer` adds text atomically at the current end, with optional
  `expected_tail`. Use actual newlines. `edit_buffer` takes an expected revision
  and character ranges. Both are undoable and leave the buffer unsaved.
- `propose_terminal_text` accepts one line and returns immediately. The agent
  should stop and let the person review it in Runyte. Approval inserts text
  without Enter; submitting is a separate human action. Status and cancellation
  tools operate on the returned proposal handle. Do not poll for approval.

Handles belong to one MCP process and one connection generation. They cannot
be shared between agents or reused after revocation, connection renewal,
eviction, or a host restart. The server retains at most eight connections and 4,096 handle
mappings. Host limits remain authoritative. Lost mutation acknowledgements
return `outcome_unknown`; never retry automatically.

The server uses MCP `2025-06-18` with bounded newline-delimited JSON on stdio,
including both structured and text tool results. It reuses the editor's private
context transport on Unix and Windows. The host owns all grants, transactional
edits and terminal approvals. Buffer and terminal text is untrusted source data.

## Migrating from Python

Change the MCP command from `runyte-context` to the Runyte binary and prepend
`mcp` to its arguments. Keep the same `--identity` and private-store environment.
Existing grants work. Restart the agent once to load the new configuration.
The old Python adapter remains available, including snapshot and viewport
tools, but retains its dynamic tool-list and agent-startup limitations.
