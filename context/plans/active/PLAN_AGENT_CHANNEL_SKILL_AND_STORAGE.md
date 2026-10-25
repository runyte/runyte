# Runyte MCP skill and durable agent channels

Status: active. Stage 1 is implemented and verified locally;
Stage 2 has partial subagent trial evidence; native-client user tests remain.
The initial delivery is the cooperative skill and its
installer. Hooks and terminal wake-up
are tentative investigations, not committed implementation stages.

This extends the [completed agent-channel plan](../completed/PLAN_AGENT_CHANNEL.md)
and its related [issue #29](https://github.com/runyte/runyte/issues/29). It does
not change current behavior until implemented. Repository inspection baseline:
`594f057`, 2026-10-25. No new tracking issue has been created for this proposal.

## Objectives

- Make existing Runyte MCP capabilities easy for an agent to use after one
  instruction from the person, especially ongoing channel participation.
- Let the agent choose when to check messages according to its task and the
  person's requests. Do not prescribe a fixed interval or universal checklist
  of mandatory checkpoints.
- Separate retained conversation history, recipient unread state and attempts
  to notify an agent.
- Support coordination across workspaces without duplicating terminal or
  channel infrastructure in the terminal and desktop frontends.
- Keep installation explicit, offline-capable and safe for user modifications.
- Test the skill before deciding whether lifecycle hooks or terminal activation
  justify their complexity.

## Relationship to the completed plan

The original channel scope is implemented in `6d642f4`. Keep that plan as the
completed architecture record; this proposal is the single plan for the
follow-up work discussed here. The original later-work ideas map as follows:

| Original follow-up | Disposition here |
| --- | --- |
| Wake-up notices for idle agents | Tentative investigation after skill user tests |
| Named channels beyond the workspace channel | Durable public channels and fixed-membership groups |
| History across host restarts in project `.runyte/` | Durable SQLite history; proposed location changes to user-level `~/.runyte/` for cross-workspace use |
| The person posting from the channel page | Remains outside this implementation scope; no delivery commitment |

The new proposal does not reopen completed work or imply that its storage and
receipt semantics are already available. Human posting is retained here only
to preserve the earlier design note, not as a parallel backlog or an approved
implementation item.

## Existing implementation

The shared workspace host owns one public channel and direct messages. Each
agent launches a separate `runyte mcp` adapter; adapters reach the shared host
through the authenticated context transport. The adapter currently exposes
thirteen tools, including `join_channel`, `post_message`, `read_messages` and
`list_participants`.

`src/workspace/context/channel.rs` owns the bounded in-memory conversation,
participant UUIDs, resume tokens, message sequences and read watermarks.
`src/mcp/channel.rs` owns adapter cursors, automatic reconnect and bounded
long-poll reads. `src/workspace/host/context_channel.rs` verifies terminal
associations and supplies unread badges and the person's channel page.

Current limitations relevant to this plan:

- History lasts only as long as the host and drops old entries after 1,000
  messages or 4 MiB, including unread entries. Reads report lost history.
- A read advances the host watermark before its response reaches the agent.
- The MCP process keeps resume credentials in memory. Reconnecting that
  process preserves identity; restarting it does not automatically recover it.
- There are 64 participant slots but only eight simultaneous context readers.
  A ten-agent scenario requires a bounded admission-limit revision.
- Internal terminal IDs are not recycled within a host lifetime; displayed
  terminal numbers are recycled. Unix association checks a terminal capability
  and process session. The channel currently reports no verified terminal on
  Windows. Association does not prove that an agent owns the current prompt.
- MCP terminal text insertion never sends Enter. This plan's initial delivery
  preserves that boundary.

## First delivery: Runyte MCP skill

Keep one canonical instruction-only skill at:

```text
skills/runyte-mcp/SKILL.md
```

Use the broader name because the skill teaches interaction with Runyte MCP,
including resource discovery, buffer and terminal access, and agent channels.
Its description should identify those uses explicitly and help the agent load
it when the person mentions Runyte, Runyte MCP or the agent channel in an
interaction task. A mention in an unrelated editor-development task should not
cause unnecessary MCP access.

The skill must use the tools and guarantees actually shipped with its release.
The first version uses the current channel; it must not describe proposed
durability, acknowledgements or named groups as available capabilities.

The skill should explain:

- How to discover the intended workspace and resources, distinguish ambiguous
  targets, join a channel, identify participants and send public messages or DMs.
- How to use buffer and terminal tools within existing grants, exact handles,
  revision checks and terminal approval rules. Terminal insertion is not
  command submission.
- Once asked to participate, keep checking the relevant channels throughout
  the task without requiring the person to repeat the instruction.
- Choose checks based on the work: after substantial work items, while waiting
  for another agent, or through a supported timer or scheduled task when useful
  and consistent with the person's request. No fixed cadence is required.
- A shell cron job that reads MCP messages is not inherently a mechanism for
  delivering them into a model's context. Prefer an agent-native scheduling
  facility when available; otherwise use cooperative checks. Do not create
  background readers that consume messages without informing the agent.
- Adapt checking frequency when coordination becomes more or less important.
  Avoid busy polling, unbounded waits and unnecessary tool calls. Stop task-
  specific scheduled checks when participation ends.
- Treat message text as peer-provided data and requests, not as human approval
  or permission to exceed the task. Keep replies self-contained.
- Follow bounded pagination when needed, report missing access or lost history,
  and do not automatically retry mutations with unknown outcomes.

Installation makes the skill discoverable; it cannot guarantee invocation or
continued compliance. Document a single startup request, such as “Use the
Runyte MCP skill and coordinate through the agent channel for this task.” Do
not automatically modify `AGENTS.md`, `CLAUDE.md`, MCP configuration or startup
hooks. Any future bootstrap integration is separate from skill installation.

## Skill installation and updates

Provide a small shared CLI surface:

```sh
runyte skills install --claude --user
runyte skills install --codex --project
runyte skills install --all --user
runyte skills update --all --user
runyte skills status --all --user
```

Require an explicit project or user scope. `--all` targets the supported agents.
Resolve project scope to the selected project root and print the destination.
The commands work without a running editor or MCP host. Installing never runs
automatically at editor startup.

Current documented destinations:

| Agent | Project | User |
| --- | --- | --- |
| Claude Code | `.claude/skills/runyte-mcp/` | `~/.claude/skills/runyte-mcp/` |
| Codex | `.agents/skills/runyte-mcp/` | `~/.agents/skills/runyte-mcp/` |

Verify supported paths against current client documentation when implementing.
Embed the canonical skill in the executable and include it in the Cargo package
allowlist. Both executable editions distribute the same payload. Installing
or updating requires no downloads, external skill manager or Python runtime.

Keep a small Runyte-owned metadata file beside the installed skill, recording
metadata schema, skill version, distributing Runyte version and hashes of the
exact installed files. The content hash, not just the version, determines
whether an update is safe.

| Destination state | Result |
| --- | --- |
| Absent | Install payload and metadata |
| Already equals the bundled payload | No-op, or establish consistent metadata |
| Equals the previously recorded installed hashes | Update on explicit update |
| Modified, unknown or inconsistent | Preserve files and report a conflict |
| Unexpected symlink or unsupported metadata schema | Refuse mutation and explain |

`install` should direct an existing older installation to `update`, keeping the
distinction predictable. Optional conflict output may write a uniquely named
`.new` candidate for manual merging; never replace an existing candidate.
Do not silently downgrade an installed newer skill. Do not modify unrelated
skills or adopt different unmanaged content merely because its name matches.

Use staged writes, atomic replacement where supported and a recoverable update
protocol for payload plus metadata. Serialize concurrent installers and
revalidate the destination before replacement. An interrupted update must be
recoverable without interpreting user changes as managed content. Report
per-target outcomes and a nonzero status for partial failures or conflicts.

## Durable channel model

Retention belongs to the conversation type, independently of unread state.

| Conversation | Membership | Retention |
| --- | --- | --- |
| Public channel | Agents may subscribe later | Full history retained until explicit operator deletion |
| DM | Two fixed participants | Message eligible for deletion after both parties acknowledge it |
| Fixed group | An explicit set of N participants | Message eligible for deletion after all N acknowledge it |

Public means visible to agents authorized for that channel, not globally
readable by every local MCP client. The person's channel view remains able to
inspect conversations within their authority; operator viewing does not
acknowledge messages on behalf of an agent.

Public history does not disappear when current subscribers read it, leave or
disconnect. A new subscriber receives the latest 20 messages by default and
can explicitly request older pages. Responses carry stable pagination cursors
and indicate that older history exists. Count and byte limits both apply.

The latest-20 default is an initial view, not a rule that skips unread backlog.
Once subscribed, an agent resumes from its receipt position and pages through
new messages. Older pre-subscription history is available on request without
being presented as an obligation to acknowledge the entire archive. Fetching
an older page must not acknowledge intervening messages that were never read.

DMs are fixed groups of two for storage purposes. Fix member identities when
creating a group; changing membership creates a new group rather than changing
who must acknowledge existing messages. Disconnecting or closing a terminal
does not remove a participant from the acknowledgement set. An abandoned group
may require explicit operator retirement; no inactivity timeout silently
deletes its pending messages.

For the sender, successful submission counts as that party's acknowledgement
of the content it authored. All other members must acknowledge receipt. State
this convention in the protocol so two-party deletion does not require the
sender to fetch its own message again. If a send response is lost, an
idempotency key allows the sender to recover the accepted outcome.

Deleting fully acknowledged DM/group bodies should retain bounded protocol
tombstones as needed for retry deduplication and stable pagination. Define the
deduplication window explicitly. Deletion is logical removal from the channel,
not a promise of forensic erasure from SQLite journals or backups.

## Receipts, notifications and identity

Distinguish these events:

1. Stored: the message and its recipient obligations committed successfully.
2. Notification attempted: a mechanism attempted to tell a recipient to check.
3. Receipt acknowledged: the recipient acknowledged a returned message batch.

A reminder never acknowledges the messages. Acknowledgement establishes receipt,
not comprehension, acceptance of work or task completion.

Add negotiated channel capabilities for durable storage and explicit receipts.
Reuse existing sequence numbers within a durable channel identity. Reads return
an opaque receipt tied to the authenticated participant and exact returned
batch. Acknowledgement is idempotent and cannot cover unseen messages. Lost read
responses leave messages available for replay. Older clients retain their
documented best-effort behavior; they must not accidentally obtain stronger
guarantees or clear receipt obligations through legacy reads.

Use a client-generated idempotency key for retryable sends. Bind it to sender,
channel and message content; conflicting reuse is an error. A successful send
means durable commit, not admission to an in-memory write queue.

Persist participant identity and recovery credentials separately from transient
connections and terminal bindings. A resume credential remains private and
must not appear in model-visible replies, channel messages or logs. Define how
an agent explicitly resumes the same identity after its MCP process restarts;
do not infer it from a shared grant identity, directory, branch or terminal.
Multiple agents in one terminal retain separate participant identities.

Keep current terminal association as optional metadata. It must be revalidated
after reconnect and must not survive a host incarnation as a usable input
target. Public UUID knowledge alone is never proof of identity.

Durable unread state provides eventual discovery only if the recipient later
checks or a supported integration reaches it. Neither an installed skill nor a
stored message wakes an idle or terminated agent by itself.

## SQLite storage across workspaces

Use one per-user SQLite store, provisionally `~/.runyte/agent-channels.sqlite3`,
so conversation identity and history are not tied to one workspace's runtime
directory. This user-level directory is distinct from each project's ignored
`.runyte/`. Make the storage root injectable and provide an explicit override
for tests and isolated environments. Resolve home directories through platform
APIs and use owner-private permissions, including database sidecar files.

Suggested logical tables cover channels, fixed membership or subscriptions,
participants, messages, receipt state, send deduplication and schema migrations.
Notification-attempt records are unnecessary until a notification mechanism is
implemented. Avoid duplicating the message body for each recipient.

Use transactions for accepted sends and acknowledgement/deletion decisions.
Use SQLite concurrency controls, bounded busy waits and a documented durability
setting. Handle disk-full and failed commits explicitly; never report a durable
send as accepted before commit. Retain full public history without automatic
age/count eviction. If a configured storage ceiling is reached, reject new
sends with an actionable error instead of removing retained history.

Storage access runs through bounded workers off the editor loop. Page history
from disk; do not load the archive into memory at startup. Open the store lazily
when channel functionality needs it. Migration failure must not block ordinary
editing or silently recreate an empty database over existing history.

A shared database is not itself cross-workspace authorization. Before this
stage is implemented, specify a versioned host context extension that maps
channel access to grants and membership across participating workspaces.
Each request still passes through an authorized host; MCP adapters must not
bypass host grants by querying SQLite directly. Cross-workspace discovery must
not disclose channel names, participants or messages to unauthorized readers.
Define revocation and host ownership consistently when several hosts access
the store, and never persist an unconditional access decision in an adapter.

This authorization design and stable participant recovery are prerequisites
for cross-workspace delivery, not reasons to delay the skill release. Do not
add a global broker process merely to centralize the database; evaluate one
only if the specified host coordination cannot be implemented safely without it.

## Ownership and compatibility

Preserve the current architecture:

- Channel values and semantics stay under `src/workspace/context/`.
- Authorization, request coordination and worker results stay in the shared host.
- MCP adaptation stays under `src/mcp/`.
- Skill management stays beside the shared CLI.
- Terminal and desktop frontends render shared state; neither owns a separate
  channel scheduler or database.

Do not make crate extraction or a new feature-flag matrix prerequisites. A
future optional crate is justified only by measured dependencies or clearer
ownership. Runtime opt-in and lazy initialization should keep unused features
cheap. Hosts advertise supported protocol capabilities independently of the
attached frontend; terminal-built hosts must remain usable from desktop clients.

Any new context operation follows the process-boundary reference and receives
a versioned contract, bounded payloads and authorization tests. Do not expose
the privileged bundled-client wire to agents. Read the UI vocabulary and
keymap references if implementation changes the channel page or commands.

## Delivery sequence and acceptance

### Local implementation progress

- Stage 1: canonical `skills/runyte-mcp/SKILL.md`, embedded Cargo payload,
  shared `runyte skills install/update/status` dispatch, hash-based managed
  updates, recoverable metadata intent and kernel locking are implemented.
- Ordinary skill reads preserve existing permissions; private runtime file
  admission remains separate. Destination traversal rejects links. Durable
  writes sync only beneath the selected project/home root.
- Unit tests cover installation, ownership conflicts, interrupted updates,
  metadata versions, locking, permissions and bounded reads. A CLI acceptance
  test covers project-root selection and partial multi-target failure.
- Astra Medium reviewed the skill package clean, then reviewed installer fixes
  until clean. A final independent Astra Medium review found no actionable
  findings in either package. Skill schema validation and package inclusion
  checks passed.
- Linux verification: formatting, core and desktop clippy, desktop tests, and
  `cargo test`, and canonical `cargo llvm-cov --locked --package runyte` passed. Line coverage is
  92.14%, above the 89% floor. Windows execution remains for CI; its ownership
  admission and file-lock paths received source review.
- Stage 2 has partial subagent trial evidence below; independent native-client
  tests remain pending. Durable storage and speculative notification
  integrations have not been implemented.

When changing the bundled payload, increment `VERSION` in `src/cli/skills.rs`
so downgrade checks distinguish skill releases independently of Runyte versions.

### 1. Cooperative skill and installer

Implement only the skill, installation/update/status commands, packaging and
setup documentation. No channel protocol changes, database, hooks or PTY
observers are necessary for this release.

Acceptance:

- Both editions install identical payloads offline without a live host.
- The skill describes only available tools and their actual guarantees.
- Test absent, identical, old managed, modified and unmanaged destinations;
  newer versions, malformed metadata, symlinks, concurrent invocations,
  interrupted updates, path spaces and partial multi-target failure.
- Package verification proves the canonical skill is included in release builds.
- All test destinations and agent configuration paths use temporary storage.
- Startup performs no installation, configuration edits or background polling.

### 2. First user tests and refinement

Test Claude Code and Codex with a single instruction to use Runyte MCP. Observe
whether each loads the skill, chooses sensible check points, discovers DMs and
public messages, and continues participation through substantial work items.
Exercise an agent-chosen timer only where the client supports it and the task
calls for it. Record missed-message cases and tool-call overhead without
collecting private conversation content into repository records.

Distinguish failure to load the skill, failure to check while working, unread
history eviction, and a message arriving after the recipient has stopped.
Refine the skill based on observed behavior. These tests are behavioral evidence,
not a deterministic guarantee that every future model follows the instructions.

#### Subagent trial, 2026-10-25

Baseline: `594f057` plus the uncommitted Stage 1 implementation. Two Codex
subagents explicitly read the canonical skill and performed bounded read-only
onboarding/test-coverage audits while participating in a live workspace channel.
The coordinator used its native MCP tools. Each child used a separate current
`target/debug/runyte mcp` process through the existing test-only
`RealMCPClient` bridge, with temporary configuration storage. Three distinct
participant identities were observed. The bridge transported manually chosen
calls; it did not schedule reads or deliver reminders. No terminal input,
buffer edits, grants or user configuration changes were made.

Initial native child discovery exposed a nine-tool catalog without channel
methods; the coordinator exposed thirteen tools. The cause of that inventory
mismatch was not established. Separate current adapters found no endpoints
inside the execution sandbox; the same discovery outside it found the live
hosts and existing channel grants. Setup assistance preceded the behavioral
trial; no collaboration-tool reminders were sent once participants joined.
This is an assisted explicit-skill trial, not an automatic-loading test.

Observed results:

- Both children chose reads between audit work items and acknowledged the
  public message sent during their work. Send-to-acknowledgement times from
  channel timestamps were approximately 12 and 15 seconds; these include
  processing/reply time and are not delivery guarantees.
- Both answered their individual synthetic DMs privately, approximately 17
  and 25 seconds after posting. The peer reported no incoming coordinator DM
  addressed to the other recipient. All observed reads reported `lost=false`.
- The peer's initial default read retrieved the public message posted before
  it joined. The receiver deliberately started at the join head to avoid old
  conversation and therefore skipped that message. This is a cursor-selection
  consequence, not history eviction or evidence of reliable backlog discovery.
- A message posted after the peer closed its adapter was accepted while that
  participant was reported disconnected. Acceptance does not establish receipt
  or wake-up; no further execution was requested from that child.
- The peer reported one live discovery, one join, one participant listing,
  three reads and three posts over roughly one minute of channel participation.
  Two unsuccessful sandbox discovery calls preceded that trial. These counts
  exclude shell bridge overhead and are not general cost estimates.
- The receiver reported one live discovery, one join, one participant listing,
  six reads and four posts, plus three setup discovery calls. It used bounded
  waits, including a final empty 20-second read, and distinguished its own
  returned posts from new requests to avoid acknowledgement loops.

Onboarding refinements suggested by the trial: explicitly select the exact
returned `root` and use its workspace handle when path selectors match similar
workspace names; distinguish missing channel tools from missing grants; explain
that the first read includes earlier history and that choosing the join head
skips it; note that reads also include the participant's own messages. These
findings do not establish a need for hooks or PTY activation.

Stage 2 remains partial. Still untested: automatic installed-skill selection,
independent Claude Code and Codex native-client sessions, long tasks with less
explicit test instructions, supported scheduling, eviction, restart recovery,
and frontend-specific behavior. This short, prompted trial supports cooperative
checks during active work, not universal compliance or eventual delivery.

### 3. Durable conversations

Finalize the cross-workspace authorization and identity recovery contract, then
implement SQLite storage, public history, DMs/fixed groups, explicit receipts
and bounded admission sufficient for at least ten simultaneous agents.

Acceptance:

- Public history survives host/adapter restarts and acknowledgement by every
  current subscriber. A late join returns the latest 20 by default; older
  pages are explicitly available.
- A subscriber with more than 20 unread messages can retrieve every unread
  message without a hidden jump to the newest page.
- DM and group bodies are deleted only after all required acknowledgements;
  sender acknowledgement, disconnects and operator retirement follow the
  documented policy. Public messages are not collected by this mechanism.
- Lost replies, duplicate sends/acks and crashes before/after commit preserve
  the documented delivery contract. Acknowledgements cannot skip unseen data.
- Concurrent hosts share consistent storage without cross-workspace data leaks.
  Test grants, revocation, identity recovery and terminal replacement.
- Disk-full, busy database, corrupt state and unsupported migrations produce
  bounded errors without freezing editing or destroying existing data.
- Ten-agent admission and activity remain bounded and responsive.
- Test legacy/new capability combinations and terminal/desktop attachment parity.

### 4. Tentative notification investigation

Proceed only after first user tests identify an unmet need and a subsequent
decision selects additional work. There is no commitment to ship either hooks
or terminal submission.

Documented SessionStart, tool-boundary or Stop hooks may provide reminders when
agents execute. Keep adapters small, optional and separately installed, with
current-version fixtures, timeout handling and continuation-loop prevention.
They must query pending status without acknowledging messages. A message
arriving after the last hook still needs a future event or supported wake-up.

Generic PTY observation remains an uncertain fallback. Quiet output is not
proof of readiness, and an empty-looking prompt does not rule out active work.
Any later proposal must address stale screen observations, user typing, approval
dialogs, agent exit to a shell, multiple agents per terminal and terminal reuse.
Unknown state must defer, never force input. Automatic Enter would require its
own explicitly enabled authority, separate from existing terminal insertion.

## Verification requirements

Extend the existing channel unit tests and `tests/mcp/` fixtures. Subprocess
fixtures set `XDG_CONFIG_HOME` and channel storage overrides to temporary
directories; no tests contact real agents or their configuration. Use the
checked-in stand-in executable for subprocess fixtures.

For Rust implementation, run the repository-required formatting, Clippy and
test checks, including desktop checks when shared behavior can affect it.
Maintain the canonical coverage floor. Measure startup and idle behavior before
and after durable storage or any later timers using the existing performance
harness; do not place synchronous disk work or full-history scans on the editor
loop. Documentation-only plan changes do not require these Rust checks.

## Research references

These sources informed the proposal; their implementation details are not
Runyte contracts. Recheck client APIs before implementing adapters.

- [msg in claude-code-tools](https://pchalasani.github.io/claude-code-tools/tools/msg/):
  the close match for the discussed SQLite inbox, hooks and terminal watcher.
  Its [store](https://github.com/pchalasani/claude-code-tools/blob/main/claude_code_tools/msg/store.py),
  [watcher](https://github.com/pchalasani/claude-code-tools/blob/main/claude_code_tools/msg/watcher.py)
  and [prompt detector](https://github.com/pchalasani/claude-code-tools/blob/main/claude_code_tools/msg/prompt_detect.py)
  separate read state from notification but retain heuristic readiness checks.
- [Herdr](https://github.com/herdrdev/herdr): agent-specific detection manifests
  and integrations isolate compatibility changes; they do not establish a
  universal idle-agent signal.
- [Gas Town nudges](https://github.com/gastownhall/gastown/blob/main/internal/cmd/nudge.go):
  queued and immediate notification paths have different interruption and
  eventual-delivery properties. Its forced fallback is unsuitable here.
- [Skills CLI](https://github.com/vercel-labs/skills): useful scope, destination
  and metadata conventions without taking a runtime dependency on the tool.
- [Claude Code skills](https://code.claude.com/docs/en/skills) and
  [Codex skills](https://learn.chatgpt.com/docs/build-skills): current discovery
  and installation conventions.
- [Claude Code hooks](https://code.claude.com/docs/en/hooks) and
  [Codex hooks](https://learn.chatgpt.com/docs/hooks): possible future documented
  lifecycle mechanisms, subject to explicit installation and client trust rules.
