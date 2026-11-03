# Agent channel

Status: completed. Implemented in `6d642f4` (Add an agent channel shared through
runyte mcp); completion verified against source and existing tests at `594f057`.
This record describes the original in-memory, workspace-scoped channel.

Tracking issue: [#29](https://github.com/runyte/runyte/issues/29), item 5.

The [follow-up proposal](../active/PLAN_AGENT_CHANNEL_SKILL_AND_STORAGE.md)
consolidates future skill, storage and notification work. Completion of this
plan does not determine the status of the broader tracking issue.

## Problem

Agents running in Runyte terminals coordinate through their providers' own
messaging, which only reaches agents from the same provider and only one
recipient at a time. A Claude Code agent cannot tell a Codex agent in the next
pane what it is doing, and an orchestrating agent cannot address everyone at
once. Every agent already reaches Runyte through `runyte mcp`, so the workspace
host can carry the conversation.

## Decisions

- **Scope.** One public channel per workspace host, plus direct messages
  between two participants, in the manner of IRC. More channels can be added
  later without changing a participant's identity or the message record.
- **Owner.** The workspace host keeps the channel in memory. It lasts as long as
  the host: a persistent session keeps it across attachments, and a standalone
  editor loses it on exit. Nothing is written under `context/` or `.runyte/`.
- **Authority.** A new grant permission, `agent_channel`, is negotiated like the
  other context capabilities. Hosts that carry the channel advertise the
  optional feature `runyte.context.channel.v1` in their hello; a client asks for
  the capability only from such a host, so a newer `runyte mcp` keeps working
  with an older editor and the reverse. The MCP catalog stays fixed; the four
  channel tools are always listed and refuse calls the grant does not cover.
- **Joining requires a statement.** An agent joins by naming its provider
  (`claude`, `codex`, ...) and its main task. Without a join it can neither read
  nor post.
- **Identity.** Each participant has:
  - a UUID assigned by the host at the first join, kept when the same MCP
    process joins again after reconnecting. The UUID is public, so the process
    proves itself with a private resume token that only its join reply carried;
  - a nickname `provider-N`, numbered per provider in the order agents joined;
  - the MCP identity the grant belongs to (verified by the host's
    authentication);
  - the MCP client's reported name and version from `initialize`;
  - the terminal it runs in, verified when possible: an integrated terminal of
    a persistent session hands its children a capability in
    `RUNYTE_PARENT_CONTEXT`; the adapter forwards it and the host accepts it
    only if it matches the terminal and the connecting process belongs to that
    terminal's session (Unix). Otherwise the terminal is unknown;
  - the location the adapter reports: the directory its MCP server started in,
    or one the agent names after moving, with the repository root, worktree
    and branch around it.
- **Changing place.** The adapter reports the location with every join and
  post. When it differs from the participant's last one, the host records a
  `moved` event naming both before the message, so readers can tell what the
  agent's earlier messages referred to.
- **Delivery.** Messages are pulled. `read_messages` returns what arrived since
  the agent's last read and can wait up to 30 seconds for something new. A
  terminal running a joined participant shows `[N unread]` in its pane title,
  so the person can see who has not caught up. A Runyte-written wake-up line
  submitted into an agent's terminal is deliberately not part of this work:
  it would be the first thing to send Enter on an agent's behalf and needs its
  own decision.
- **The person's view.** `:agent-channel` opens a generated read-only page,
  `[agent channel]`, listing participants and recent messages, direct ones
  included, and refreshing as messages arrive. The person does not post from
  it yet.
- **Bounds.** At most 64 known participants, of which the host's eight readers
  can be connected at once; when full, the participant that disconnected
  longest ago gives way. At most 1,000 retained messages and 4 MiB as encoded
  per host; a message holds at most 16 KiB of text and no control characters
  other than newline and tab. A read returns at most 200 messages and about
  256 KiB as encoded, which an MCP reply carries twice within its 2 MiB frame.
  Older messages are dropped first, and a read that asks for dropped ones says
  so.
- **Trust.** Message text is untrusted data written by another agent. The
  adapter marks it as such, and nothing in Runyte executes or interprets it.

## Wire

Methods, all under `agent_channel`:

| Method | Parameters | Result |
| --- | --- | --- |
| `channel.join` | `provider`, `task`, optional `client {name, version}`, `location`, optional `terminal {terminal, capability}`, optional `resume` to rejoin | the participant record, its `resume` token and `head` |
| `channel.post` | `text`, optional `to` (participant UUID), `location` | `seq` |
| `channel.read` | `since`, `limit` | `messages`, `next`, `lost` |
| `channel.who` | none | `participants` |

`location` is `{directory, repository, worktree, branch}`; every field but
`directory` may be null. A message record is `{seq, at_ms, kind, from,
from_nick, to, to_nick, text, location}` with `kind` one of `joined`,
`updated`, `moved` and `message`.

## MCP tools

`join_channel`, `post_message`, `read_messages` and `list_participants`, taking
an optional `workspace`. The adapter fills in the client, location and
terminal itself, remembers the participant per workspace, and continues reads
from where the last one stopped.

## Completion evidence

The implementation is in `src/workspace/context/channel.rs`,
`src/workspace/host/context_channel.rs` and `src/mcp/channel.rs`. Current usage
and limits are documented in `docs/mcp.md` and `docs/user-guide.md`.

Existing regression coverage includes:

- `src/workspace/context/tests/channel.rs`: participant identity and resume,
  required join fields, recorded location changes, DM visibility, unread state,
  bounded retention and read sizes, and departed-participant eviction.
- `src/workspace/host/tests/context_access.rs`:
  `agents_share_the_channel_and_a_verified_terminal_counts_what_is_unread`
  exercises host requests, verified association, unread indicators and the
  person's channel page.
- `src/mcp/tests/host.rs`:
  `channel_tools_join_once_continue_reads_and_rejoin_after_a_reconnect` and
  `a_channel_only_grant_keeps_its_connection_between_calls` exercise adapter
  cursors, reconnect and channel-only grants.

The lifecycle update is documentation-only; these tests were inspected, not
rerun. The former later-work list is accounted for in the follow-up proposal;
it was outside this plan's implemented scope.
