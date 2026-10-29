## Agent access to workspace context

Runyte includes an MCP server for agents to find and read integrated terminals,
unsaved buffers, edit buffers, and propose terminal text. Start
it with `runyte mcp --identity agent`. It uses stdio and private local workspace
connections, with no Python installation. See the [MCP setup guide](../mcp.md).

### Setting it up

1. Configure the agent to launch your Runyte binary with arguments
   `["mcp", "--identity", "agent"]`.
2. In each workspace to share, run `:mcp agent` using that same identity.
3. Choose permissions and select **Apply permissions** with Tab then Enter.

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
| `1` | Read terminal output |
| `2` | Editor-context read permission; MCP exposes buffer reads |
| `3` | Edit and append to buffers; also enables buffer reads |
| `4` | Propose terminal text for review; also enables terminal reads |
| `5` | Skip review: insert proposed text directly, still without Enter; also enables `4` and `1` |
| `6` | Join the agent channel: read and post messages with other agents |
| `7` | Accept channel wakes: configured notifications or explicit terminal backup (which sends Enter); also enables `6` |
| `r` | Remember this exact workspace's grant |
| `n` | Next known identity; discards unapplied choices |
| `x` | Revoke the selected identity immediately |
| `j` / `k`, `PgUp` / `PgDn`, `gg` / `G` | Scroll longer reviews by line, by screen, or to the first or last line |
| `Tab` then Enter | Select **Apply permissions** and apply |
| `Esc` | Close without applying |

Turning off a permission also turns off the ones that need it: turning off
terminal reads turns off proposals and direct insertion.
The remembered checkbox reflects the current grant when reopened. Without it,
the grant lasts until the owning editor or persistent host exits. Applying a
change disconnects existing readers, so agents must rediscover their targets.

Granting, changing, and revoking need physical input from the active Runyte
frontend, apart from the defaults in your configuration described under [Default
grants](agent-access-to-workspace-context.md#default-grants). Repeated input, pasted text, macros, and MCP clients
cannot approve access or terminal text. Each terminal proposal needs its own
native approval unless the grant includes `5`, and text is always inserted
**without Enter**.

**Skipping review (`5`).** With direct insertion granted, a proposal for a
terminal that has bracketed paste on, as shells and agent prompts do, is typed
in as a paste as soon as it arrives, behind any text already queued there, and a
notification records which identity inserted how many characters into which
terminal. The text still cannot contain line breaks or control characters, and
Enter is never sent. A terminal whose program has not turned on bracketed paste,
such as a pager, a single-key prompt or an editor in normal mode, would read the
text as keystrokes, so its proposals are still reviewed. Runyte sees only the
terminal's own mode, so inside tmux, zellij or screen, which turn bracketed
paste on for themselves and pass text to the pane as keystrokes when its program
did not ask for paste, this check does not protect you: do not grant `5` to
agents that type into a multiplexer. Even as a paste, text lands in whatever
input is open, so grant it only to agents you trust, in workspaces where you
expect them to type. The notifications are a recent record, not an audit log:
they share the bounded notification history. A Runyte release that predates this
permission reads a grant containing it as invalid and, because it stops at that
record, ignores every remembered grant in that workspace until the grant is
applied again or revoked.

The server works with ide-mode and mux-mode workspaces on Linux, macOS,
and Windows. Detached hosts support content reads and buffer edits, but cannot
approve terminal insertion until a native frontend is attached.

### Default grants

To let agents read without opening `:mcp` in every project, name the
permissions in your configuration:

```yaml
mcp:
  default_grants:
    agent: [terminal_read, editor_context_read]
```

A workspace that starts with no grant record for `agent` grants it these
permissions for as long as it runs. Its own decisions take precedence: a grant
applied with Remember replaces the default there, and revoking with `x`, or
applying with nothing selected, records a revocation that keeps the default off
in that workspace until you apply a grant again. A grant applied without
Remember lasts for the session and leaves the default in place for next time.
`:mcp` says when a grant comes from the default. Revocations made before this
release removed the record instead of keeping one, so revoke again where a new
default should stay off. Revocation records stay in the private store until a
grant replaces them.

Defaults use the same names as the context profile. Write permissions need their
read permission (`buffer_edit` needs `editor_context_read`, `terminal_propose`
needs `terminal_read`). Skipping review (`5`) cannot be a default: it is decided
per workspace. Runyte reads the setting when a workspace host starts, so a
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

With permission `6`, agents of any provider share a conversation through the
workspace host: a public channel and direct messages between two agents. An
agent joins by stating its provider and main task; Runyte adds a stable id, a
nickname such as `codex-1`, its MCP client, where it works (repository, worktree
and branch), and, inside a persistent session's integrated terminal, the
terminal it runs in, checked rather than taken on its word. When the branch it
works on changes, or it reports another directory, the channel says where it
moved from.

A terminal whose agent has unread messages shows `[N unread]` in its title.
Sender-requested wakes default to terminal-free delivery through an explicitly
configured Claude channel or Codex queue adapter. Permission `7` is still
required. See [notification setup](../mcp.md#nudges). Legacy terminal wakes remain
an explicit backup and can answer an agent's pending dialog with Enter.

When an agent's terminal is unknown, use `:agent-bind codex-1 2` in the
workspace containing its terminal, then review the workspace and terminal title
and approve with Tab and Enter. The number is the pane title's `#2`; terminal
numbers in other workspaces are unrelated. The binding captures the underlying
terminal, so closing it and reusing its number never redirects nudges. Use
`:agent-bind codex-1 none` to remove the binding. Authenticated reconnects keep
it; reapplying permissions clears it. See [terminal binding](../mcp.md#binding-a-participant-to-a-terminal).

`:agent-channel` opens `[agent channel]`, a read-only page listing the
participants and the recent messages, direct ones included, which refreshes as
messages arrive. Messages last as long as the workspace host. The [MCP
guide](../mcp.md#agent-channel) describes the tools and limits.

**Nudges (`7`).** A sender can request a fixed wake reminder with a post.
The updated MCP adapter defaults to notifications, which need an explicitly
configured recipient and permission `7`, but no terminal. A `queued` result
means the host accepted the notification, not that the agent acted. See
[Claude and Codex setup](../mcp.md#nudges). Unsupported receivers never cause an
automatic terminal fallback.

**Explicit terminal backup.** A sender selecting `nudge_transport: "terminal"`
uses the legacy paste followed by Enter. Runyte types only the fixed line
`Runyte: you have new messages on the agent channel.` The recipient needs `7`,
a connected participant, and a live, automatically verified or natively bound
terminal reading bracketed pastes. Peer message text never reaches that terminal.

A nudge waits while you may be typing there: while the line holds text you
typed or pasted, or an agent's proposal inserted, and nothing submitted it with
Enter or abandoned it with `Ctrl-c`; for 30 seconds after your last keystroke in
that terminal; and while a terminal proposal for it is waiting. Alt-Enter does
not count as submitting, since prompts use it for a new line, but Runyte cannot
tell when a program continues a line itself, as after a trailing `\` or an
unclosed quote, and then counts Enter as submitting. If anything reaches the
terminal between the nudge's line and its Enter, the Enter is not sent and the
line waits for you like your own text. A terminal gets at most one nudge until
its agent reads the channel again. Each nudge leaves a notification naming the
sender, the participant and the terminal.

Permission `7` also allows the explicit terminal backup, so grant it only where
that unexpected Enter is acceptable. If the agent is showing
an approval prompt when the nudge arrives, Enter may choose the highlighted
option, which is often approval. The waits above make this less likely but do
not prevent it; Runyte does not read the screen to guess. Inside tmux, zellij or
screen the paste mode is the multiplexer's, as with `5`. `7` cannot be given by
`mcp.default_grants`. A remembered grant that includes `7` cannot be read by an
earlier Runyte release, which then loads no remembered grants for that
workspace; reapply them there with `:mcp` after going back.

An opt-in [SQLite channel trial](../mcp.md#experimental-sqlite-channel-trial)
adds durable public and fixed-group conversations through a separate MCP tool.
It requires explicit storage and private profiles; this page and terminal unread
indicators continue to show the in-memory channel.

### How agents use it

Install the optional Runyte MCP skill with `runyte skills install --all --user`
for Claude Code and Codex, or select one with `--claude` or `--codex`. Use
`--project` for a project-local installation. Then ask the agent once to use
the skill and coordinate through the channel for its task. It chooses when to
check according to the work; an idle agent learns of a message only when a
sender [nudges](agent-access-to-workspace-context.md#agent-channel) it and you granted `7`.
See [skill installation and safe updates](../mcp.md#runyte-mcp-skill).

- Start with `find_resources`, optionally selecting a workspace and searching
  buffer or terminal names and contents. Use the returned exact handle with
  `read_buffer`, `read_terminal`, `append_buffer`, `edit_buffer`, or
  `propose_terminal_text`. Repeated names remain separate candidates.
- `list_workspaces` checks permissions without reading source text.
  `find_resources` can read content to match a query; `match_in="name"`
  restricts matching to names. Search is bounded and may return partial results.
- `terminal_proposal_status` and `cancel_terminal_proposal` operate on the
  proposal handle. Let the person review a proposal rather than polling for
  approval.
- `join_channel`, `post_message`, `read_messages` and `list_participants`
  use the [agent channel](agent-access-to-workspace-context.md#agent-channel).
- These are the thirteen MCP tools. Pane listing, selection and viewport reads,
  and immutable snapshots are not exposed by the MCP adapter.
- Granted detached persistent workspaces and running ide-mode editors are
  discoverable without attaching a TUI or changing focus. Terminal approval,
  and direct insertion, need an attached native frontend.
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
- MCP reads default to 80 recent rows, bounded by 64 KiB and 65,536 visited
  cells. `max_rows` allows up to 1,000 rows; `region="screen"` reads the live
  screen. Reads are live and may change between calls.
- Reading does not move review, acknowledge unread output, send input, or
  change the child's lifetime.
- Returned content is untrusted source material, not instructions to the
  receiving agent.

### Buffer edits

With the edit grant:

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

### Terminal proposals

With the proposal grant, an agent can ask to type literal text into a
terminal. You review it in **Review terminal text**:

1. The target terminal stays visible above the review. The exact proposed value
   comes first in the scrolling content, followed by the requester, target,
   workspace, untrusted reason (if any), and recent output.
2. Scroll with the editor's Normal-mode page motions: `Ctrl-u` / `Ctrl-d`
   for half pages, `Ctrl-b` / `Ctrl-f` or PageUp / PageDown for full pages,
   and `gg` / `ge` for the beginning / end. Home / End are also supported.
   The mouse wheel scrolls over the review content. Scrolling never changes
   the selected action. The dialog uses 80% of the editor width and 60% of
   its height, with an 80 × 22 minimum where space permits; smaller areas
   use the available space. Text wraps to the available width.
   Every line must actually reach the screen before approval is enabled;
   jumping past lines does not count as reading them. Rewrapping after a
   width change requires reviewing the newly laid-out text again.
3. **Reject** is selected at first. `Down` / `Up` always choose between it
   and **Insert text (no Enter)**, even for long proposals. `Tab` /
   `Shift-Tab` also switch choices. The choices remain visible while scrolling.
4. One fresh physical `Enter` inserts the text. That Enter belongs to the
   overlay. **Submit separately in the terminal.**

**Making hidden characters visible.** In the proposed value, spaces show as
`·`, backslashes are doubled, and non-ASCII characters are escaped. Labels keep
printable Unicode and spaces but escape control, zero-width, bidirectional,
and space-like characters. A short proposal fits without scrolling. The overlay
needs at least 80 × 24 cells.

**What cannot approve:** macros, pasted keys, repeated input, and the context
API.

**What a proposal can contain:**

- single-line text of at most 4 KiB;
- no line breaks, control characters, escape sequences, or Unicode line or
  paragraph separators.

Runyte never appends Enter, clears existing input, or sends cursor keys.
Insertion uses the child's current input position, and recent output does not
prove the input line is empty. Some programs act on printable input at once,
so approval to insert is no guarantee that the child waits for submission.

**Limits and cancellation:**

- Each reader may queue four proposals, sixteen per host, expiring after two
  minutes.
- Existing prompts keep their input ownership. A detached target refuses
  proposals until you attach.
- Changes to terminal input or input modes invalidate an approval.
- Disconnect, revocation, exit, and attachment loss cancel queued work before
  writing begins.
- Delivery means the bytes reached the PTY, not that the child accepted a
  command. A partial write or lost response is uncertain and must never be
  retried automatically.

<!-- navigation -->
[Contents](README.md) · [Previous: Plugins](plugins.md) · [Next: Diagnostics and logging](diagnostics-and-logging.md)
