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
env_vars = ["XDG_RUNTIME_DIR", "XDG_CACHE_HOME", "RUNYTE_PARENT_CONTEXT"]
```

The server must see the same `XDG_RUNTIME_DIR` and `XDG_CACHE_HOME` as the
editor. Runyte lists only workspaces registered from the same runtime and cache
environment, so that editors started for tests never appear beside real ones.
A client that starts servers with a reduced environment must pass both
variables through; `env_vars` does this in the TOML form above. When they are
missing, discovery returns no workspaces, `hidden_workspaces` counts the live
ones it left out, and `discovery_help` names the variables.

### Codex and Claude Code

Add Runyte to both clients with the same `agent` identity:

```sh
codex mcp add runyte -- /path/to/runyte mcp --identity agent
claude mcp add --scope user --transport stdio runyte -- /path/to/runyte mcp --identity agent
```

Codex starts MCP servers with a reduced environment, and `codex mcp add` has
no option to pass a variable through by name. After adding the server, add
the `env_vars` line to the `[mcp_servers.runyte]` table that
`~/.codex/config.toml` now contains:

```toml
env_vars = ["XDG_RUNTIME_DIR", "XDG_CACHE_HOME", "RUNYTE_PARENT_CONTEXT"]
```

Another client that does not pass these variables through needs the same
addition in its own configuration; `hidden_workspaces` in a `list_workspaces`
reply shows when it does.

`RUNYTE_PARENT_CONTEXT` enables automatic terminal verification when the MCP
server runs inside the integrated terminal's process session. A daemon-launched
server may have no current terminal context and belongs to another process
session; passing the variable alone cannot establish its terminal. Do not copy a
saved value into configuration. After the agent joins, use
[explicit terminal binding](#binding-a-participant-to-a-terminal) in Runyte.

Check the saved configurations with `codex mcp list` and `claude mcp list`.
Restart any already-running agent session after adding the server. Each client
starts its own MCP process, but both use the same Runyte permission grant for
`agent` in a workspace. Grant it with `:mcp agent` as described below.

Use the absolute path to your Runyte binary. Windows uses the same
arguments with an absolute path to `runyte.exe`. `runyte mcp --help` describes
the options. `--timeout 2` sets the per-host deadline (0.1–10 seconds).

## Runyte MCP skill

Runyte bundles an optional skill for Claude Code and Codex. It teaches resource
access and ongoing channel participation. Install it explicitly, without a
running editor or network access:

```sh
runyte skills install --all --user
runyte skills status --all --user
runyte skills update --all --user
```

Use `--claude` or `--codex` instead of `--all` for one agent, and `--project`
instead of `--user` for the current project root (a Git repository or a directory
initialized by Runyte). The scope is required. Project installations go into
`.claude/skills/runyte-mcp/` and `.agents/skills/runyte-mcp/`; user installations
use those paths under the operating-system account's home directory.

Ask once: “Use the Runyte MCP skill and coordinate through the agent channel for
this task.” The agent chooses useful times to check according to the task and
your requests, including supported timers when appropriate. Installation makes
the skill discoverable; it does not wake stopped agents or guarantee that an
agent follows it. MCP setup and grants above are still required.

`install` creates a missing skill or recognizes identical content. Use `update`
to replace an older managed copy. Both preserve user-modified or unknown content
and report conflicts; no force-overwrite option is provided. Status is read-only.
Updates use recorded content hashes, refuse downgrades, and recover interrupted
managed writes on the next install/update. Keep `.runyte-skill.json` beside the
skill: it records ownership and update state. Other skill files and agent
configuration are left untouched. Multi-agent operations report each target and
return failure if any target fails. Nothing installs automatically on startup.

## Permissions

Open `:mcp agent` in each workspace to share. The identity must match
`--identity`; its label does not authenticate an agent executable. Use separate
identities, such as `codex` and `claude`, when permissions should differ.

Toggle terminal reads (`1`), editor-context reads (`2`), buffer edits (`3`),
terminal proposals (`4`), optionally direct insertion without review (`5`),
the [agent channel](#agent-channel) (`6`), and optionally accepting
[channel nudges](#nudges) (`7`). Optionally remember the grant with
`r`, then Tab and Enter to **Apply permissions**.
Write permissions automatically enable the corresponding read permission.
`:mcp` reopens the current grant, lifetime and connected-reader count; `n`
cycles known identities, and `x` revokes the selected identity. Readers connect
when the agent performs discovery; tool listing alone leaves the count at zero.
Revocation drops readers and pending proposals. `:context-access` remains an alias.

To grant permissions in every workspace without opening `:mcp` in each, set
`mcp.default_grants` in the configuration; see the
[user guide](user-guide/agent-access-to-workspace-context.md#default-grants).
A workspace's own grant or revocation takes precedence over the default.

**Starting the agent before granting is fine.** The fourteen tools stay available;
a tool's presence is not permission to use it. Calls check the target's native
grant. After a permission change, run `find_resources` again. Healthy searches
preserve handles. Changed permissions renew the connection and require fresh
handles.

The default identity is `agent`. Without Remember, a grant lasts until the
owning editor or persistent host exits. Private credentials and remembered
grants stay in Runyte's account cache, outside the project. `RUNYTE_CONTEXT_HOME`
can select an absolute private store shared by the editor and MCP process.
See the [user guide](user-guide/agent-access-to-workspace-context.md#what-a-grant-covers) for platform locations.

## Tools

The catalog always contains these fourteen tools. The experimental
`durable_channel` tool requires separately configured SQLite storage and an
explicit private profile (see below).

| Tool | Purpose |
| --- | --- |
| `durable_channel` | Experimental durable conversations and explicit receipts; requires [separate opt-in setup](#experimental-sqlite-channel-trial). |
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
| `post_message` | Post to the channel, or directly to one participant, optionally nudging named participants. |
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
beyond the current environment. When nothing in the current environment is
found, the reply carries `hidden_workspaces` and `discovery_help` if live
workspaces from another environment exist. `list_workspaces` checks identity
and grants without reading source content.

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
  only from a process in that terminal's session (Unix). A native terminal
  binding can also identify it; otherwise the terminal is unknown;
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
messages when it next reads, when the person asks it to, or when a sender
nudges it.

### Binding a participant to a terminal

For an agent whose `join_channel` reports `terminal: null`, including an MCP
server started by a Codex daemon, open `:agent-channel` to find its nickname.
In the **workspace containing that agent's terminal**, run:

```text
:agent-bind codex-1 2
```

`2` is the terminal number shown as `#2` in its pane title, not an internal ID.
Numbers are local to each workspace: another workspace's `#2` is unrelated.
The native confirmation shows the workspace, participant, identity, task and
terminal title. Review it, then Tab and Enter to apply. Participant names and
tasks are self-reported, so select the terminal where that agent is running.
Only physical frontend input can approve; MCP calls, paste and macros cannot.
This does not grant nudge permission: accepting nudges still needs `7` in `:mcp`.

Use `:agent-bind codex-1 none` to remove the binding and suppress automatic
claims for that participant. A binding survives authenticated reconnection with
the same participant's resume token. A new participant must be bound again.
Reapplying or revoking its identity's permissions clears native bindings;
exiting the workspace host clears them too. Closing or exiting the target
terminal makes the binding unusable; a new terminal reusing its number does not
inherit it. Run the command again to choose a replacement. A participant that
reconnects while a confirmation is open requires a fresh review.

### Nudges

`post_message` can name up to 63 participants in `nudge`. A public post can
name any connected peer; a DM can name only its recipient. Unknown, repeated,
or self names reject the entire post. The recipient needs permission `7`
(which requires `6`) in `:mcp`; it cannot be a configured default.

The default `nudge_transport` is `notification`. A notification carries a fixed
Runyte-authored reminder and the adapter's workspace handle, never peer message
text. The recipient then calls `read_messages`. No terminal or terminal binding
is needed. Configure each recipient adapter explicitly as described below.
Unconfigured or unsupported recipients return `unavailable`; an older host
returns `unsupported`. Neither case silently sends terminal input.

| State | Meaning |
| --- | --- |
| `queued` | The host accepted a notification; it does not prove the client received it or started a turn |
| `already_pending` | A wake is outstanding until that participant reads the channel |
| `unavailable` | The recipient lacks a live configured receiver or permission |
| `sent` | Explicit terminal backup queued its fixed line; Enter follows subject to input guards |
| `held` | Explicit terminal backup is waiting for its typing/proposal guards |

Notifications are bounded and coalesced per recipient connection. Reading the
channel clears the outstanding notification; disconnecting or changing grants
invalidates that connection. Messages remain available even if delivery fails.
Delivery is not retried automatically: a timeout may have delivered. Already
emitted client notifications cannot be recalled on revocation. These wakes
apply to the workspace channel, not the deferred durable SQLite channel.

#### Claude Code

Configure the MCP command as:

```text
runyte mcp --identity claude --channel-notifications claude
```

Start Claude interactively with the configured MCP server name:

```sh
claude --dangerously-load-development-channels server:runyte
```

Accept Claude's development-channel confirmation. This is a research-preview
client requirement for custom channels, separate from Runyte's grant. The
adapter advertises `experimental.claude/channel` and emits
`notifications/claude/channel`. It does not advertise the permission-relay
capability and never sends permission responses. Claude queues events while
busy; pending approvals still require the person. Channels require supported
Anthropic authentication, feature availability and organization policy. The
custom-channel development flag does not enable this in `claude -p` or the
Agent SDK. A disabled channel can silently discard events.

Contract: [Claude channels reference](https://code.claude.com/docs/en/channels-reference)
and [channel availability](https://code.claude.com/docs/en/channels).

#### Codex CLI

Codex has no documented equivalent MCP channel wake notification. Runyte uses
its client-specific queue command instead; **Codex CLI 0.164.0 is the tested
version**. Configure the adapter for one exact persisted recipient thread:

```text
runyte mcp --identity codex --channel-notifications codex --codex-thread THREAD_UUID
```

Create or choose a persisted Codex thread first, obtain its exact UUID from
Codex's thread metadata, and configure this adapter for that thread before
resuming it. Replace `THREAD_UUID` with that UUID. This is trusted launch
configuration, never a peer-supplied tool argument. Do not reuse a
thread-bound adapter configuration for another agent or thread. The adapter
must inherit the same `CODEX_HOME` and have `codex` on `PATH` so it addresses
the matching managed daemon. Ephemeral threads cannot receive queued input.
There is no automatic thread inference from a daemon's environment.

Runyte executes `codex queue --thread UUID --message FIXED_REMINDER` with an
argument vector, discarded output and a five-second deadline. The reminder
includes the adapter's workspace handle so a multi-workspace recipient reads
the right channel. The bounded worker preserves MCP request processing while
the CLI runs. Idle threads start a turn; input waits behind an active command
approval. A failure is reported on adapter stderr and leaves the channel
message for a later read; it does not cause a retry or terminal fallback.

The [Codex App Server documentation](https://learn.chatgpt.com/docs/app-server)
documents thread and approval behavior but does not currently document the
queue interface. This integration is experimental and version-tested, not a
portable MCP guarantee. Recheck it when upgrading Codex.

#### Explicit terminal backup

Set `nudge_transport: "terminal"` in `post_message` to choose the legacy path.
This requires the recipient's permission `7`, a verified or natively bound
live terminal, and bracketed paste. Runyte pastes one fixed line and then Enter.
It holds while a draft or proposal is outstanding and for 30 seconds after
physical typing. Other input between paste and Enter cancels submission.

**Enter can answer a pending agent dialog.** Only choose the terminal backup
when that risk is acceptable and the recipient is known to be at its input
prompt. The notification transport never uses terminal input as a fallback.
Old context clients retain their original terminal semantics; the updated MCP
adapter explicitly selects notifications by default.

## Migrating from Python

Change the MCP command from `runyte-context` to the Runyte binary and prepend
`mcp` to its arguments. Keep the same `--identity` and private-store environment.
Existing grants work. Restart the MCP client to load the new configuration.
The Python adapter has been removed; no separate package is needed.

### Experimental SQLite channel trial

`durable_channel` is an opt-in experimental tool alongside the existing channel.
The existing `join_channel`, `post_message`, `read_messages`, terminal unread
indicator and `[agent channel]` page still use the host's in-memory conversation.
They do not read or acknowledge SQLite messages.

For an isolated trial, start the host with `RUNYTE_CHANNEL_HOME` set to an
absolute private storage directory whose parent exists. The host advertises
`runyte.context.channel.durable.v1` only when configured. SQLite runs on a bounded
worker, not on the editor loop. An unavailable directory fails the operation;
there is no fallback database. With the variable unset, the host starts no
worker and opens no channel database.

Start each trial MCP adapter with `--channel-profile /absolute/profile.json`.
The profile must already exist in a private directory as an owner-private file,
containing `workspace` (the exact 32-character workspace ID), `store` (SHA-256 of
the configured storage path's native encoding), and `secret` (64 random hex
characters). Unix paths use raw bytes; Windows paths use UTF-16LE. The adapter
holds an exclusive kernel lock for its lifetime. Use one distinct profile per
agent; reopening the same profile recovers that participant. Never put profile
contents in prompts or channel messages. General profile installation and
recovery management remain unfinished; the isolated acceptance fixture provisions
these files. Do not replace a live profile file while it is locked.

The tool supports `register`, `resume`, `create`, `subscribe`, `post`, `read`,
`history` and `acknowledge`. Use `register` explicitly once, then `resume` after
restart. Public conversations retain history; subscribing initially exposes the
latest 20 messages, and `history` with `before` requests older pages. `read`
returns oldest outstanding messages, 20 by default. Reads return exact-batch
receipts and never acknowledge automatically. `acknowledge` clears only the
returned batch. Fixed groups include the creator and supplied participant IDs;
the creator implicitly acknowledges its own sends, and bodies are deleted only
when every other member acknowledges. There is no participant/channel discovery
yet, so trial participants exchange their returned IDs explicitly.

`post` requires an original 64-hex `nonce`, Unix-millisecond `issued_ms`, text,
and immutable `directory`. Preserve every argument when retrying an uncertain
send. The seven-day retry window rejects expired keys rather than risking a
second send after deduplication state expires. Receipts expire after 24 hours;
read again if a receipt expires. Store failures return a bounded generic error.

Every request requires the live workspace's `agent_channel` grant, negotiated
feature, matching profile store binding and current channel policy. Cancellation
skips queued jobs and suppresses replies. A transaction already admitted may
commit during revocation: a suppressed reply is an uncertain outcome, not a
rollback. Retry with the original key after authorization is restored. Profiles
are workspace-local in this trial; there is no agent tool for changing policy
or granting cross-workspace access. Native policy management, durable channel
presentation, ten-agent transport admission and general deployment remain
Stage 3 work. The skill and wake-up behavior are unchanged.

The real-host acceptance test is:

```sh
RUNYTE_CONTEXT_TEST_BINARY="$PWD/target/debug/runyte" \
  python3 -m unittest discover -s tests/mcp -p test_durable_channel.py -v
```

It uses temporary workspace, storage, profile, configuration and discovery
roots, and does not touch a running personal channel.
