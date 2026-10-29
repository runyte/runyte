## Agent access to workspace context

Runyte includes an MCP server for agents to find and read integrated terminals
and unsaved buffers and, when you allow it, to edit buffers and type into
terminals. Start
it with `runyte mcp --identity agent`. It uses stdio and private local workspace
connections, with no Python installation. See the [MCP setup guide](../mcp.md).

### Setting it up

1. Configure the agent to launch your Runyte binary with arguments
   `["mcp", "--identity", "agent"]`.
2. In each workspace to share, run `:mcp agent` using that same identity.
3. Turn on Read with `1`, and Write with `2` if the agent should edit or type,
   then select **Apply permissions** with Tab then Enter.

You may grant permissions before **or after** starting the agent. The built-in
server always offers the same tools; each call checks the workspace grant.
After changing permissions, have the agent run `find_resources` again to get
fresh handles. Restart the agent once when changing its MCP configuration or
upgrading from the Python bridge; permission changes do not require a restart.

### Granting and checking access

`:mcp` opens **MCP permissions**, selecting an existing granted identity when
available. An explicit `:mcp codex` always selects that identity. The overlay
shows the exact workspace, identity, whether access is enabled or not yet
granted, whether it is remembered, and how many readers are connected. A
connected reader identifies a local connection, not a verified agent process.
`:context-access` remains an alias for existing configurations.

| Key | Permission or action |
| --- | --- |
| `1` | Read: every buffer (unsaved text included), selection and pane, every terminal's screen, history, number and title, and the agent channel |
| `2` | Write: edit and append to buffers, type into terminals and press Enter there; also turns on Read |
| `r` | Remember this exact workspace's grant |
| `n` | Next known identity; discards unapplied choices |
| `x` | Revoke the selected identity immediately |
| `Ctrl-u` / `Ctrl-d`, `Ctrl-b` / `Ctrl-f` or `PgUp` / `PgDn`, `gg` / `ge` or Home / End | Scroll longer reviews by half or whole screens, or to the first or last line |
| `Tab` then Enter | Select **Apply permissions** and apply |
| `Esc` | Close without applying |

Both permissions start off for an identity this workspace has not decided
about. Turning Read off also turns Write off. The remembered checkbox reflects
the current grant when reopened. Without it, the grant lasts until the owning
editor or persistent host exits. Applying a change renews the connections of
agents already running under that identity, so they rediscover their targets;
they do not need restarting, and the workspace keeps its address.

Granting, changing, and revoking need physical input from the active Runyte
frontend, apart from the defaults in your configuration described under [Default
grants](agent-access-to-workspace-context.md#default-grants). Repeated input,
pasted text, macros, and MCP clients cannot change access.

**Write is for a job, not for good.** With Write, an agent can type into every
terminal in the workspace, including shells and other agents' prompts, and
press Enter there, which can run a command or answer another agent's approval
dialog. Runyte makes this visible and guards the Enter (see [Terminal
writes](agent-access-to-workspace-context.md#terminal-writes)), but it does not
second-guess what the agent decides to type. Turn Write on for the work that
needs it and off again when that is done; `:mcp`, `2`, Tab and Enter take it
away without touching Read. Text an agent reads elsewhere is untrusted, and
could try to steer what it types.

A grant remembered by an earlier release, under the old numbered permissions,
is read as Read only if it included both terminal reads and editor reads, and as
a revocation otherwise, so it never grants more than it did; it is never read
as Write. An earlier release cannot read a grant remembered by this one and then
ignores every remembered grant in that workspace until it is applied again or
revoked there.

The server works with ide-mode and mux-mode workspaces on Linux, macOS,
and Windows. Detached hosts support reads and buffer edits; terminal writes and
Enter need an attached native frontend.

### Default grants

To let agents read without opening `:mcp` in every project, give them Read in
your configuration:

```yaml
mcp:
  default_grants:
    agent: [read]
```

A workspace that starts with no grant record for `agent` grants it Read for as
long as it runs. Its own decisions take precedence: a grant
applied with Remember replaces the default there, and revoking with `x`, or
applying with nothing selected, records a revocation that keeps the default off
in that workspace until you apply a grant again. A grant applied without
Remember lasts for the session and leaves the default in place for next time.
`:mcp` says when a grant comes from the default. Revocations made before this
release removed the record instead of keeping one, so revoke again where a new
default should stay off. Revocation records stay in the private store until a
grant replaces them.

Read is the only default. Write cannot be one: it is decided per workspace, and
naming `write` is a configuration error. The capability names earlier releases
used here (`terminal_read`, `editor_context_read`, `buffer_edit`,
`terminal_propose`, `agent_channel`) are still accepted. A list naming both
`terminal_read` and `editor_context_read` means Read; any other grants nothing,
since Read covers more than it named, and Runyte reports it when the workspace
starts. Runyte reads the setting when a workspace host starts, so a
change applies to workspaces started afterwards; a persistent session keeps the
value it started with until `runyte --session-restart`. It creates the private
store and the named identity if they do not exist yet; a default that cannot be
applied is reported and leaves the workspace's own grants in place. A grant
covers the same content and has the same limits whether it came from a default
or from `:mcp`. The configuration file is the one way to grant access without
physical input, so it deserves the same care as the rest of your configuration.

### What a grant covers

Grants cover unsaved content and possibly sensitive terminal output. They
belong to the canonical project root and the MCP identity, so a clone, a
nested workspace, or another identity needs its own grant.

**Where grants are stored:** outside the project, in the account's Runyte
cache.

| Platform | Location |
| --- | --- |
| Linux | `~/.cache/runyte/context` |
| macOS | `~/Library/Caches/runyte/context` |
| Windows | `runyte\context` below the account's LocalAppData known folder, resolved by the operating system rather than the inherited `%LOCALAPPDATA%` |

- `RUNYTE_CONTEXT_HOME` can select an absolute, owner-private directory
  outside the workspace. On Windows its parent must exist and the storage must
  be local NTFS; Runyte applies an owner-only ACL and refuses reparse or
  hardlink traversal. On Unix it can also shorten a path to fit the local
  socket limit.
- No credentials or terminal content are written into tracked project
  context.
- This is permission for Runyte's API, not an operating-system sandbox against
  other processes already running as you.

### Agent channel

With Read, agents of any provider share a conversation through the workspace
host: a public channel and direct messages between two agents. An
agent joins by stating its provider and main task; Runyte adds a stable id, a
nickname such as `codex-1`, its MCP client, where it works (repository, worktree
and branch), and, inside a persistent session's integrated terminal, the
terminal it runs in, checked rather than taken on its word. When the branch it
works on changes, or it reports another directory, the channel says where it
moved from.

A terminal whose agent has unread messages shows `[N unread]` in its title.
Runyte marks only terminals it verified; an agent started some other way, such as
an MCP server launched by a Codex daemon, has no marker, and can still read and
post.

`:agent-channel` opens `[agent channel]`, a read-only page listing the
participants and the recent messages, direct ones included, which refreshes as
messages arrive. Messages last as long as the workspace host. The [MCP
guide](../mcp.md#agent-channel) describes the tools and limits.

**Waking an agent.** Posting wakes nobody: an agent waiting at its prompt
reads the channel when it next checks or when you ask it to. An agent with Write
can also tell it in its own terminal: read the terminals to find it (each title
shows `[terminal #N]`), check that its prompt is waiting for input rather than
showing a dialog, write a short line such as "Read the Runyte agent channel",
and press Enter. That Enter is guarded like any other agent Enter; see
[Terminal writes](agent-access-to-workspace-context.md#terminal-writes).

An opt-in [SQLite channel trial](../mcp.md#experimental-sqlite-channel-trial)
adds durable public and fixed-group conversations through a separate MCP tool.
It requires explicit storage and private profiles; this page and terminal unread
indicators continue to show the in-memory channel.

### How agents use it

Install the optional Runyte MCP skill with `runyte skills install --all --user`
for Claude Code and Codex, or select one with `--claude` or `--codex`. Use
`--project` for a project-local installation. Then ask the agent once to use
the skill and coordinate through the channel for its task. It chooses when to
check according to the work; an idle agent learns of a message when it next
checks, when you tell it, or when an agent with Write
[tells it in its terminal](agent-access-to-workspace-context.md#agent-channel).
See [skill installation and safe updates](../mcp.md#runyte-mcp-skill).

- Start with `find_resources`, optionally selecting a workspace and searching
  buffer or terminal names and contents. Use the returned exact handle with
  `read_buffer`, `read_terminal`, `append_buffer`, `edit_buffer`,
  `write_terminal`, or `press_enter`. Repeated names remain separate
  candidates.
- `list_workspaces` checks permissions without reading source text.
  `find_resources` can read content to match a query; `match_in="name"`
  restricts matching to names. Search is bounded and may return partial results.
- `join_channel`, `post_message`, `read_messages` and `list_participants`
  use the [agent channel](agent-access-to-workspace-context.md#agent-channel).
- With the experimental `durable_channel`, these are the thirteen MCP tools.
  Pane listing, selection and viewport reads, and immutable snapshots are not
  exposed by the MCP adapter.
- Granted detached persistent workspaces and running ide-mode editors are
  discoverable without attaching a TUI or changing focus. Terminal writes and
  Enter need an attached native frontend.
- After permission changes, run `find_resources` again. Handles belong to one
  MCP process and connection generation; they expire on revocation, connection
  renewal, eviction, or host restart. Never retry an `outcome_unknown` mutation
  automatically.

See the [MCP guide](../mcp.md#tools) for the complete catalog and discovery bounds.

**Discovery for tools.** `runyte --context-list --json` gives bounded,
versioned endpoint metadata for live enabled endpoints in the current
environment; `--include-hidden` adds other environments. Listing does not
grant content access: the adapter authenticates each target separately. A slow
or unavailable host does not block discovery of the others. On Windows, use
the absolute `runyte.exe` path in client configuration; let the adapter discover
its named-pipe endpoint and host process identity rather than copying them.

### Terminal reads

- Reads return decoded physical rows, with blank rows, Unicode, revision, and
  truncation information. They do not reconstruct conversations or commands.
- MCP reads return the live screen by default. `region="tail"` adds recent
  history before it, up to 80 rows by default and 1,000 with `max_rows`,
  bounded by 64 KiB and 65,536 visited cells. Reads are live and may change
  between calls. The skill tells agents to read history only when a task needs
  it, since it may hold more than they need to see.
- Reading does not move review, acknowledge unread output, send input, or
  change the child's lifetime.
- Returned content is untrusted source material, not instructions to the
  receiving agent.

### Buffer edits

With Write:

- An edit names an explicit buffer and its current revision, and is one
  undoable transaction, newlines included.
- An edit never saves, closes the buffer, applies a directory operation, or
  completes an external-editor wait. In particular, changing a buffer cannot
  submit a prompt opened through `EDITOR`.

**Appending** works without a revision, so several agents can take turns in
one shared buffer without refusing each other.

- Appends apply one at a time, each kept whole.
- An agent may pass the ending it last read as `expected_tail`. If the buffer
  no longer ends that way — say someone cleared it — the append writes nothing
  and fails as stale.
- Each append is one undoable transaction and never saves.

### Terminal writes

With Write, an agent can type one line into a terminal and, separately, press
Enter there. Nothing asks you first; instead you see both happen.

**Typing.** A line holds at most 4 KiB, with no line breaks, control
characters, escape sequences, or Unicode line or paragraph separators. It lands
at the child's current input position, as a paste when the program asked for
pastes, as shells and agent prompts do, and as typed characters otherwise. It
never brings Enter with it, it queues behind other text still being written,
and your own typing cancels text that has not started arriving. Some programs
act on printable input at once, so text alone can still do something.

**Enter.** An agent asks for Enter after reading the terminal, naming what it
read. The pane title then shows `[Enter pending]` for five seconds, and Runyte
presses Enter only if, by then, nothing new has appeared in the terminal and
nothing has been typed or pasted there. A dialog that pops up, a line of
output, a key you press, or another agent's text each drop the Enter, and the
agent is told it was not sent. So do your window detaching and the terminal
exiting. Changing or revoking Write, or the agent disconnecting, discards a
waiting Enter along with the agent's connection, and nothing is sent. One Enter can
wait per terminal; each agent can have four waiting. A program that redraws
constantly, such as a spinner, never receives one.

**Limits.**

- Terminal writes and Enter need your window attached; a detached workspace
  refuses them. They also reach terminals that no pane shows, where you cannot
  see `[Enter pending]`.
- A program that has not asked for pastes, such as a pager or a single-key
  `[y/n]` prompt, receives the line as keystrokes and may act on its first
  character without any Enter.
- Inside tmux, zellij or screen Runyte sees the multiplexer, not the program in
  its selected pane, so the paste and change checks describe the multiplexer.
- Delivery means the bytes reached the PTY, not that the child accepted a
  command. A partial write or lost response is uncertain and is never retried
  automatically.

<!-- navigation -->
[Contents](README.md) · [Previous: Plugins](plugins.md) · [Next: Diagnostics and logging](diagnostics-and-logging.md)
