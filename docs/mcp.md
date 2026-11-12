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
editor. Runyte lists only workspaces from the same runtime and cache
environment. Editors started for tests never appear beside real ones. A client
that starts servers with a reduced environment must pass both variables
through. `env_vars` does this in the TOML form above. When they are missing,
discovery returns no workspaces, `hidden_workspaces` counts the live ones it
left out, and `discovery_help` names the variables.

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
addition in its own configuration. `hidden_workspaces` in a `list_workspaces`
reply shows when it does.

`RUNYTE_PARENT_CONTEXT` enables automatic terminal verification when the MCP
server runs inside the integrated terminal's process session. A daemon-launched
server may have no current terminal context and belongs to another process
session. Passing the variable alone cannot establish its terminal. Do not copy a
saved value into configuration. Such an agent's channel participant then has
no known terminal, which only means its pane shows no `[N unread]` marker. It
can still read, post and be reached through its terminal.

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

Use `--claude` or `--codex` instead of `--all` for one agent. Use `--project`
instead of `--user` for the current project root (a Git repository or a
directory initialized by Runyte). The scope is required. Project installations
go into `.claude/skills/runyte-mcp/` and `.agents/skills/runyte-mcp/`. User
installations use those paths under the operating-system account's home
directory.

Ask once: “Use the Runyte MCP skill and coordinate through the agent channel
for this task.” The agent decides when to check, based on the task and your
requests. It can use timers where they are supported. Installation makes the
skill discoverable. It does not wake stopped agents or guarantee that an agent
follows it. MCP setup and grants above are still required.

`install` creates a missing skill or recognizes identical content. Use `update`
to replace an older managed copy. Both preserve user-modified or unknown content
and report conflicts. No force-overwrite option is provided. Status is read-only.
Updates use recorded content hashes, refuse downgrades, and recover interrupted
managed writes on the next install/update. Keep `.runyte-skill.json` beside the
skill: it records ownership and update state. Other skill files and agent
configuration are left untouched. Multi-agent operations report each target and
return failure if any target fails. Nothing installs automatically on startup.

## Permissions

Open `:mcp agent` in each workspace to share. The identity must match
`--identity`. Its label does not authenticate an agent executable. Use separate
identities, such as `codex` and `claude`, when permissions should differ.

There are two permissions, and both start off:

| Key | Permission | What the identity's agents may do in this workspace |
| --- | --- | --- |
| `1` | Read | Read buffers outside the private context store (unsaved text included), their selection and pane, every terminal's screen, history, number and title, and take part in the [agent channel](#agent-channel), direct messages included |
| `2` | Write | Also edit and append to buffers, [type into any terminal and press Enter there](#writing-to-terminals) |

Write needs Read: turning Write on turns Read on, and turning Read off turns
Write off. Optionally remember the grant with `r`, then Tab and Enter to
**Apply permissions**. `:mcp` reopens the current grant, its lifetime and the
connected reader count. `n` cycles known identities and `x` revokes the
selected one. Readers connect when the agent performs discovery. Tool listing
alone leaves the count at zero. `:context-access` remains an alias.

**Grant Write for a job, then take it away.** Write reaches every terminal in
the workspace: shells, and other agents' prompts and approval dialogs. A change
applies at once to agents that are already running. Their connections are
renewed under the new grant, and they find their targets again with
`find_resources`. The workspace stays where it was, so no agent has to be
restarted. Text an agent reads in one terminal is untrusted and could try to
steer what it types into another. That is one more reason to keep Write
short-lived.

To grant Read in every workspace without opening `:mcp` in each, set
`mcp.default_grants` in the configuration. See the
[user guide](user-guide/agent-access-to-workspace-context.md#default-grants).
Write cannot be a default. A workspace's own grant or revocation takes
precedence over the default.

**Starting the agent before granting is fine.** The thirteen tools stay available.
A tool's presence is not permission to use it. Calls check the target's native
grant. After a permission change, run `find_resources` again. Healthy searches
preserve handles. Changed permissions renew the connection and require fresh
handles. `list_workspaces` reports each workspace's `permissions` (`read`,
`write`) beside the capability names the connection negotiated.

The default identity is `agent`. Without Remember, a grant lasts until the
owning editor or persistent host exits. Private credentials and remembered
grants stay in Runyte's account cache. The standard store may sit inside an
exact account-home workspace. Runyte excludes it from file finding, content
search and agent buffer, pane and snapshot reads. Other workspaces require the
store outside the project. `RUNYTE_CONTEXT_HOME` can select an absolute private
store shared by the editor and MCP process, but an explicit override does not
receive the home-workspace exception.
See the [user guide](user-guide/agent-access-to-workspace-context.md#what-a-grant-covers) for platform locations.
A grant remembered by an earlier release is read as Read only if it included
both terminal and editor reads, and as a revocation otherwise. Never as Write.
A release from before read and write cannot read a grant remembered by this
one, and then loads no remembered grants for that workspace.

## Tools

The catalog always contains these thirteen tools. The experimental
`durable_channel` tool requires separately configured SQLite storage and an
explicit private profile (see below).

| Tool | Purpose |
| --- | --- |
| `durable_channel` | Experimental durable conversations and explicit receipts; requires [separate opt-in setup](#experimental-sqlite-channel-trial). |
| `find_resources` | Discover buffers and terminals by name or content, with exact handles and revisions. |
| `list_workspaces` | Inspect workspace access without reading source content. |
| `read_buffer` | Read unsaved buffer text at a revision. |
| `read_terminal` | Read the live terminal screen, or recent history with it. |
| `append_buffer` | Append in one unsaved, undoable transaction. |
| `edit_buffer` | Replace character ranges at an expected revision. |
| `write_terminal` | Type one literal line into a terminal, and submit it with `enter=true`. |
| `press_enter` | Press Enter alone in a terminal, only if nothing changed there since the agent read it. |
| `join_channel` | Join the workspace's agent channel, stating provider and task. |
| `post_message` | Post to the channel, or directly to one participant. |
| `read_messages` | Read what arrived since the last read, optionally waiting for it. |
| `list_participants` | See who is in the channel, where they work and in which terminal. |

There are no MCP tools for pane listing, selections, viewports, or immutable
buffer/terminal snapshots. Those operations remain in the underlying host
context protocol. The MCP adapter does not expose arbitrary editor commands,
file saving, or arbitrary keys: Enter is the only key an agent can press.

## Finding the right target

Ask the agent to search contents as well as titles:

> Find the terminal showing Claude in this project and read its latest output.

The usual sequence is two calls:

1. `find_resources(workspace="/projects/example", query="claude", kind="terminal")`
2. `read_terminal(terminal=<returned handle>)` or, with Write,
   `write_terminal(terminal=<returned handle>, text="...")`

When you can see the terminal, its number is the shortest way to name it:

> Write `ls` to terminal #2.

A query of `#2`, `terminal 2`, `[terminal #2]` or a whole pasted title such as
`[terminal #2] codex` finds the terminal whose title shows `#2`. It checks each
searched workspace and reads no output. Pass `workspace` to get exactly one.
The number belongs to a running terminal. It is released when the terminal's
program exits or the terminal closes, and the next new terminal may take it. So
the agent writes through the returned handle. If `#2` is replaced in the
meantime, the old handle is refused and can't reach the new terminal. If no
running terminal shows the number, the search says so and returns no terminal.
A bare `2`, and `#2` with `kind="buffer"` or `match_in="content"`, are ordinary
fuzzy text. You can still find an issue reference such as `#29` in a buffer.

`find_resources` uses the editor's fuzzy matcher over names and content. `clde`
can match `Claude Code` even when the terminal is named `shell`. Results
include match excerpts, the workspace, exact handles and buffer revisions.
Several matches remain several candidates: the server never picks a write
target by fuzzy similarity. A query is optional. Omitting it lists resources.
`match_in="name"` skips content reads. `match_in="content"` only matches text.
Matching uses smart case, as the editor does.

Search has limits: four workspaces per page, eight buffers and eight terminals
per workspace by default (up to 32 each), the first 8,192 characters of each
buffer, and up to 80 recent terminal rows. Follow `next_workspace_offset` and
the per-workspace/per-kind `next_offset`. Use `content_from` for a later buffer
window. A three-second content-search budget returns partial progress.
Discovery and authentication have their own bounded deadlines. Empty results do
not prove that text is absent. No filesystem scan runs, and source text is not
cached between calls.

Workspace selectors accept a project path fragment or returned workspace
handle. Omitting one searches the discovered authorized workspaces. It never
uses current editor focus. `include_hidden=true` explicitly broadens discovery
beyond the current environment. When nothing in the current environment is
found, the reply carries `hidden_workspaces` and `discovery_help` if live
workspaces from another environment exist. `list_workspaces` checks identity
and grants without reading source content.

## Reading and writing

- `read_buffer` accepts just a discovered buffer handle for its first 16,384
  characters at the discovered revision. Optional `from`, `to`, and
  `expected_revision` give exact Unicode-character ranges. Stale reads require
  rediscovery. `read_terminal` reads the live screen by default.
  `region="tail"` adds up to 80 rows of history before it, for when a task
  needs what scrolled away.
- `append_buffer` adds text atomically at the current end, with optional
  `expected_tail`. Use actual newlines. `edit_buffer` takes an expected revision
  and character ranges. Both are undoable and leave the buffer unsaved.
- `write_terminal` and `press_enter` need Write. See
  [writing to terminals](#writing-to-terminals).

Handles belong to one MCP process and one connection generation. They cannot
be shared between agents or reused after revocation, connection renewal,
eviction, or a host restart. The server retains at most eight connections and 4,096 handle
mappings. Host limits remain authoritative. Lost mutation acknowledgements
return `outcome_unknown`. Never retry automatically.

The server uses MCP `2025-06-18` with bounded newline-delimited JSON on stdio,
including both structured and text tool results. It reuses the editor's private
context transport on Unix and Windows. The host owns all grants, transactional
edits and terminal writes. Buffer and terminal text is untrusted source data.

## Writing to terminals

With Write, an agent can type into any terminal in the workspace and press
Enter there. That can be a shell, a test runner, or another agent's prompt. The
person sees both happen. Runyte does not let an agent type where the person is
typing, because Enter answers whatever the terminal is showing at that moment
and a line typed into the middle of someone else's would be both of theirs.

1. **Read first.** `read_terminal` shows what is on screen: a shell prompt, an
   agent's input box, or a dialog asking for approval. Its pane title,
   `[terminal #N] ...`, tells agents apart. Write only where a prompt is
   waiting for this input.
2. **Write and submit.** `write_terminal` with `enter=true` types one line of
   at most 4 KiB, with no newlines or other controls, where the cursor is, and
   presses Enter as soon as the line has been written. It arrives as a
   bracketed paste when the program asked for pastes, as shells and agent
   prompts do, and as typed characters otherwise. It queues behind any other
   text still being written. The call returns the outcome of its Enter, shown
   below. Without `enter`, the line is typed and left there; the reply is
   `queued`.

`press_enter` presses Enter alone, for a line that is already there. It takes
the `revision` from the agent's latest `read_terminal`, which is refused at
once as `stale` if the terminal has changed since. Otherwise Enter is pressed
straight away, after one pass of the editor's event loop, if nothing new
appeared and nothing was typed or pasted there meanwhile.

**While the person types.** For ten seconds after the person last typed,
pasted, or undid a paste in a terminal, both tools refuse it as `busy`, and
the error says how long is left. The agent waits that long, reads the terminal
again, and decides whether it is still safe to write there. Moving or scrolling
the pointer over the pane does not count as typing, though in a program that
asked for pointer reports those reports are input: they still cancel a line
not yet written and drop a waiting Enter.

| State | Meaning |
| --- | --- |
| `sent` | Enter was pressed |
| `stale` | Someone typed there before the Enter, the terminal changed since `press_enter`'s revision, or the line was only partly written; Enter was not pressed |
| `cancelled` | The person's window detached, the terminal exited, or the line was cancelled before it was written; Enter was not pressed |

A key the person presses before a line's Enter drops it, and with it any of the
line not yet written. Another agent's text does too. The line's own echo does
not: it is expected output. A line the program has not taken within four
seconds, because it stopped reading its input, is withdrawn with its Enter, or
left without one if writing had begun. `[Enter pending]` shows in the pane
title while an Enter waits.

If Write is changed or revoked, or the connection drops, while an Enter waits,
the Enter is discarded and the tool fails with `outcome_unknown`. The change
may have come just after Enter was pressed. The agent finds the terminal again
with `find_resources` and reads it. It must not retry.

One Enter may wait per terminal. Each reader may have four waiting, the host
sixteen. Terminal writes and Enter work only while the person's window is
attached, and text still queued when it detaches is cancelled. A host from an
earlier release cannot submit a line with `enter=true`, and the tool says so
rather than typing the line alone; there, write the line, read the terminal,
and call `press_enter`.

Writes reach every terminal in the workspace, including ones no pane shows,
where the person cannot see them happen. A program that has not asked for
pastes, such as a pager or a single-key `[y/n]` prompt, gets the line as
keystrokes. It may act on the first character without any Enter.

The only key an agent can press is Enter. Inside tmux, zellij or screen, Runyte
sees the multiplexer. Revisions and pastes describe the multiplexer, not the
program in the selected pane.

## Agent channel

Agents from different providers can talk to each other through Runyte. Each
workspace host carries one channel: every agent that joins it reads the public
conversation, and any two can exchange direct messages. It is like an IRC
channel shared by the Claude Code, Codex and other agents working in the
workspace's terminals.

Read (`1` in `:mcp`, or `read` in `mcp.default_grants`) includes the channel.
An agent then joins with `join_channel`, naming its provider (`claude`,
`codex`, ...) and its main task. It can't read or post until it has joined.
Runyte identifies every participant by:

- a UUID, kept when the same MCP process joins again after a reconnect (the
  process proves itself with a private token, never shown to the agent), and
  a nickname such as `claude-1`, numbered per provider in joining order;
- the MCP identity whose grant admitted it, and the client name and version
  its MCP client reported;
- its terminal, which Runyte verifies: an integrated terminal of a persistent
  session gives its children a capability, and the host accepts it only from a
  process in that terminal's session (Unix). Otherwise the terminal is unknown;
- where it works, reported with every join and message: the directory its MCP
  server started in, or the absolute `directory` it last passed to
  `join_channel` or `post_message`, plus the repository, worktree and branch.
  When that changes, the channel records a `moved` entry naming both places
  before the next message, so earlier messages keep their meaning.

`read_messages` continues from where the agent's last read stopped. A first
read starts with what was said before it joined, and `wait_seconds` (up to 30)
waits for something new. Messages are untrusted text written by other agents. A
terminal whose agent has messages waiting shows `[N unread]` in its pane title.
`:agent-channel` opens `[agent channel]`, a read-only page of participants and
recent messages, including direct ones. It refreshes as messages arrive.

The channel lasts as long as the workspace host: a persistent session keeps it
across attachments, and nothing is written to disk. Eight agents can be
connected at once (the host's reader limit) and 64 known. When it is full, the
one that left longest ago gives way. It keeps the newest 1,000 messages (4 MiB).
A message holds at most 16 KiB and a read returns at most about 256 KiB.

### Waking another agent

Posting wakes nobody. An idle agent learns of new messages when it next reads,
when you ask it to, or when another agent with Write tells it in its terminal.
To do that, the agent reads the terminals to find the recipient's
(`list_participants` shows the terminal where Runyte verified it). It checks
that the prompt is waiting for input and not showing a dialog. It writes a
short line such as `Read the Runyte agent channel: codex-1 sent you a review`
with `enter=true`. Do this only for an agent that must act
and is idle at its prompt. A busy agent reads when it next checks.

## Migrating from Python

Change the MCP command from `runyte-context` to the Runyte binary and prepend
`mcp` to its arguments. Keep the same `--identity` and private-store environment.
Existing grants work. Restart the MCP client to load the new configuration.
The Python adapter has been removed. No separate package is needed.

### Experimental SQLite channel trial

`durable_channel` is an opt-in experimental tool alongside the existing channel.
The existing `join_channel`, `post_message`, `read_messages`, terminal unread
indicator and `[agent channel]` page still use the host's in-memory conversation.
They do not read or acknowledge SQLite messages.

For an isolated trial, start the host with `RUNYTE_CHANNEL_HOME` set to an
absolute private storage directory whose parent exists. The host advertises
`runyte.context.channel.durable.v1` only when configured. SQLite runs on a bounded
worker, not on the editor loop. An unavailable directory fails the operation.
There is no fallback database. With the variable unset, the host starts no
worker and opens no channel database.

Start each trial MCP adapter with `--channel-profile /absolute/profile.json`.
The profile must already exist as an owner-private file in a private directory.
It contains `workspace` (the exact 32-character workspace ID), `store` (SHA-256
of the configured storage path's native encoding), and `secret` (64 random hex
characters). Unix paths use raw bytes. Windows paths use UTF-16LE. The adapter
holds an exclusive kernel lock for its lifetime. Use one distinct profile per
agent. Reopening the same profile recovers that participant. Never put profile
contents in prompts or channel messages. General profile installation and
recovery management remain unfinished. The isolated acceptance fixture
provisions these files. Do not replace a live profile file while it is locked.

The tool supports `register`, `resume`, `create`, `subscribe`, `post`, `read`,
`history` and `acknowledge`. Use `register` explicitly once, then `resume`
after restart. Public conversations keep history. Subscribing initially exposes
the latest 20 messages, and `history` with `before` requests older pages.
`read` returns oldest outstanding messages, 20 by default. Reads return
exact-batch receipts and never acknowledge automatically. `acknowledge` clears
only the returned batch. Fixed groups include the creator and supplied
participant IDs. The creator automatically acknowledges its own sends. Bodies
are deleted only when every other member acknowledges. There is no
participant/channel discovery yet, so trial participants exchange their
returned IDs explicitly.

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
are workspace-local in this trial. There is no agent tool for changing policy
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
