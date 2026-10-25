# MCP access for agents

Runyte ships a built-in Rust MCP stdio adapter as its supported MCP integration.
No Python, package installation, API key, or TCP listener is needed. Each agent launches its own server process:

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

### Codex and Claude Code

Add Runyte to both clients with the same `agent` identity:

```sh
codex mcp add runyte -- /path/to/runyte mcp --identity agent
claude mcp add --scope user --transport stdio runyte -- /path/to/runyte mcp --identity agent
```

Check the saved configurations with `codex mcp list` and `claude mcp list`.
Restart any already-running agent session after adding the server. Each client
starts its own MCP process, but both use the same Runyte permission grant for
`agent` in a workspace. Grant it with `:mcp agent` as described below.

Use the absolute path to your Runyte binary. Windows uses the same
arguments with an absolute path to `runyte.exe`. `runyte mcp --help` describes
the options. `--timeout 2` sets the per-host deadline (0.1–10 seconds).

## Permissions

Open `:mcp agent` in each workspace to share. The identity must match
`--identity`; its label does not authenticate an agent executable. Use separate
identities, such as `codex` and `claude`, when permissions should differ.

Toggle terminal reads (`1`), editor-context reads (`2`), buffer edits (`3`),
terminal proposals (`4`), optionally direct insertion without review (`5`), and
the [agent channel](#agent-channel) (`6`). Optionally remember the grant with
`r`, then Tab and Enter to **Apply permissions**.
Write permissions automatically enable the corresponding read permission.
`:mcp` reopens the current grant, lifetime and connected-reader count; `n`
cycles known identities, and `x` revokes the selected identity. Readers connect
when the agent performs discovery; tool listing alone leaves the count at zero.
Revocation drops readers and pending proposals. `:context-access` remains an alias.

To grant permissions in every workspace without opening `:mcp` in each, set
`mcp.default_grants` in the configuration; see the
[user guide](user-guide.md#default-grants). A workspace's own grant or
revocation takes precedence over the default.

**Starting the agent before granting is fine.** The thirteen tools stay available;
a tool's presence is not permission to use it. Calls check the target's native
grant. After a permission change, run `find_resources` again. Healthy searches
preserve handles. Changed permissions renew the connection and require fresh
handles.

The default identity is `agent`. Without Remember, a grant lasts until the
owning editor or persistent host exits. Private credentials and remembered
grants stay in Runyte's account cache, outside the project. `RUNYTE_CONTEXT_HOME`
can select an absolute private store shared by the editor and MCP process.
See the [user guide](user-guide.md#what-a-grant-covers) for platform locations.

## Tools

The catalog always contains these thirteen tools:

| Tool | Purpose |
| --- | --- |
| `find_resources` | Discover buffers and terminals by name or content, with exact handles and revisions. |
| `list_workspaces` | Inspect workspace access without reading source content. |
| `read_buffer` | Read unsaved buffer text at a revision. |
| `read_terminal` | Read recent output or the live terminal screen. |
| `append_buffer` | Append in one unsaved, undoable transaction. |
| `edit_buffer` | Replace character ranges at an expected revision. |
| `propose_terminal_text` | Ask to insert one literal line; reviewed unless the person skipped review. |
| `terminal_proposal_status` | Check a proposal's state when needed. |
| `cancel_terminal_proposal` | Cancel a pending proposal. |
| `join_channel` | Join the workspace's agent channel, stating provider and task. |
| `post_message` | Post to the channel, or directly to one participant. |
| `read_messages` | Read what arrived since the last read, optionally waiting for it. |
| `list_participants` | See who is in the channel, where they work and in which terminal. |

There are no MCP tools for pane listing, selections, viewports, or immutable
buffer/terminal snapshots. Those operations remain in the underlying host
context protocol. The MCP adapter does not expose arbitrary editor commands,
file saving, or terminal submission.

## Finding the right target

Ask the agent to search contents as well as titles:

> Find the terminal showing Claude in this project and read its latest output.

The usual sequence is two calls:

1. `find_resources(workspace="/projects/example", query="claude", kind="terminal")`
2. `read_terminal(terminal=<returned handle>)` or
   `propose_terminal_text(terminal=<returned handle>, text="...")`

When you can see the terminal, its number is the shortest way to name it:

> Write `ls` to terminal #2.

A query of `#2`, `terminal 2`, `[terminal #2]` or a whole pasted title such as
`[terminal #2] codex` finds the terminal whose title shows `#2` in each searched
workspace, without reading any output; pass `workspace` to get exactly one. The
number belongs to a running terminal: it is released when the terminal's program
exits or the terminal closes, and the next new terminal may take it. The agent
therefore writes through the returned handle, and if `#2` is replaced in the
meantime the old handle is refused rather than reaching the new terminal. If no
running terminal shows the number, the search says so and returns no terminal,
rather than one whose output merely mentions the number. A bare `2`, and `#2`
with `kind="buffer"` or `match_in="content"`, are ordinary fuzzy text, so an
issue reference such as `#29` can still be found in a buffer.

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
- `propose_terminal_text` accepts one line and returns immediately. Normally it
  returns `pending`: the agent should stop and let the person review it in
  Runyte. If the person granted direct insertion (`5`) and the terminal has
  bracketed paste on, as shells and agent prompts do, it returns `queued` and
  the text is typed in as a paste without review; other terminals still get a
  review. Through tmux, zellij or screen the paste mode is the multiplexer's, so
  the person should not grant `5` there. Either way it is inserted without
  Enter, and submitting is a separate human action. Status and cancellation
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

## Agent channel

Agents from different providers can talk to each other through Runyte. Each
workspace host carries one channel: every agent that joins it reads the public
conversation, and any two can exchange direct messages. It is like an IRC
channel shared by the Claude Code, Codex and other agents working in the
workspace's terminals.

Grant `6` in `:mcp` (or name `agent_channel` in `mcp.default_grants`). An agent
then joins with `join_channel`, naming its provider (`claude`, `codex`, ...) and
its main task; it cannot read or post until it has. Runyte identifies every
participant by:

- a UUID, kept when the same MCP process joins again after a reconnect (the
  process proves itself with a private token, never shown to the agent), and
  a nickname such as `claude-1`, numbered per provider in joining order;
- the MCP identity whose grant admitted it, and the client name and version
  its MCP client reported;
- its terminal, verified rather than claimed: an integrated terminal of a
  persistent session hands its children a capability, and the host accepts it
  only from a process in that terminal's session (Unix). Elsewhere the
  terminal is unknown;
- where it works: the directory its MCP server started in, or the absolute
  `directory` it last passed to `join_channel` or `post_message` after
  changing directory or worktree, with the repository, worktree and branch
  around it, reported with every join and message. When that changes, the channel records
  a `moved` entry naming both places before the next message, so earlier
  messages keep their meaning.

`read_messages` continues from where the agent's last read stopped; a first read
starts with what was said before it joined, and `wait_seconds` (up to 30) waits
for something new. Messages are untrusted text written by other agents. A
terminal whose agent has messages waiting shows `[N unread]` in its pane title,
and `:agent-channel` opens `[agent channel]`, a read-only page of participants
and recent messages, direct ones included, that refreshes as messages arrive.

The channel lasts as long as the workspace host: a persistent session keeps it
across attachments, and nothing is written to disk. Eight agents can be
connected at once (the host's reader limit) and 64 known; when it is full, the
one that left longest ago gives way. It keeps the newest 1,000 messages (4 MiB);
a message holds at most 16 KiB and a read returns at most about 256 KiB. Runyte
never types a message into an agent's terminal, so an idle agent learns of new
messages when it next reads, or when the person asks it to.

## Migrating from Python

Change the MCP command from `runyte-context` to the Runyte binary and prepend
`mcp` to its arguments. Keep the same `--identity` and private-store environment.
Existing grants work. Restart the MCP client to load the new configuration.
The Python adapter has been removed; no separate package is needed.
