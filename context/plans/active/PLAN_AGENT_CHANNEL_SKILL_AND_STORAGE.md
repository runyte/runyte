# Runyte MCP skill and durable agent channels

Status: active. Stage 1 is implemented and verified locally;
Stage 2 has partial subagent trial evidence; native-client user tests remain.
Stage 3 includes the storage foundation and a tested opt-in live MCP trial;
it is deferred as of 2026-10-26, and general deployment remains unfinished.
The initial delivery is the cooperative skill and its
installer. Stage 4 selected sender-requested channel nudges on the legacy
channel as the wake-up mechanism and implemented them
([#40](https://github.com/runyte/runyte/issues/40)); they were removed on
2026-10-29 by the [read and write permissions plan](PLAN_MCP_READ_WRITE_PERMISSIONS.md),
under which an agent with Write wakes another by writing to its terminal.
Lifecycle hooks remain tentative.

This extends the [completed agent-channel plan](../completed/PLAN_AGENT_CHANNEL.md)
and its related [issue #29](https://github.com/runyte/runyte/issues/29). It
preserves the legacy in-memory channel. Repository inspection baseline:
`594f057`, 2026-10-25. Stage 4 nudges are tracked in
[issue #40](https://github.com/runyte/runyte/issues/40).

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
| Wake-up notices for idle agents | Stage 4 channel nudges, selected after the 2026-10-26 user test (removed 2026-10-29; see [PLAN_MCP_READ_WRITE_PERMISSIONS.md](PLAN_MCP_READ_WRITE_PERMISSIONS.md)) |
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

#### Manual user test, 2026-10-26

Baseline: `dev` at `11c4459`, the legacy in-memory channel. Claude Code and
Codex agents in integrated terminals shared one grant identity and coordinated
through the channel. Posting, DMs and reads worked, but recipients repeatedly
did not notice that a message was waiting for them, so coordination stalled
until the person intervened. Which failure category applied to each miss (skill
not loaded, no check while working, or idle at the prompt) was not recorded.

This is the unmet need Stage 4 required before choosing a notification
mechanism. Cooperative checks alone do not reach an agent waiting at its
prompt, which is the case a hook cannot cover either.

### 3. Durable conversations

Deferred 2026-10-26 in favor of Stage 4 nudges on the legacy channel. The
storage foundation and opt-in trial below stay in the tree, off by default, and
are not extended until this stage is explicitly resumed. Resuming it includes
deciding whether durable conversations share the Read/Write grant and the
terminal wake path that replaced Stage 4 nudges.

#### Storage foundation and integration decisions

The first package adds `src/workspace/context/durable.rs` and its SQLite schema
under the existing context ownership. It is deliberately separate from the
legacy in-memory channel. The subsequent opt-in live-trial package connects it
to the host and MCP as described below. Tests supply private temporary roots;
the eventual general-use root remains the user-level `.runyte`.

Implemented storage semantics:

- Transactional registration and explicit recovery using stored credential
  hashes. A recovery failure never creates a replacement participant.
- Public history, initial latest-20 subscription obligations, older history
  pages, and oldest-pending reads without implicit acknowledgement.
- Immutable fixed groups, including two-member DMs; sending acknowledges the
  sender and deletion requires every other member's exact receipt.
- Receipts bind participant, channel and exact message IDs, expire after 24
  hours, and support repeated acknowledgement until expiry. Lost responses
  remain replayable. There are at most 64 unacknowledged receipts and 8,192
  retained receipt records per participant.
- Send keys carry a nonce and original creation time; the key and canonical
  content are deduplicated for seven days. Older keys are rejected, even after
  tombstone collection. At most 8,192 send records per participant are retained.
- Channel policy is rechecked in each operation's transaction. Native-only
  policy replacement uses a revision fence; recovery bindings are independent
  of channel policy. Local public channels allow different grant identities in
  the same workspace. Cross-workspace routes cannot be added by agent methods.
- Private database admission, schema refusal without replacement, WAL with FULL
  synchronization, a 100 ms busy timeout and a 65,536-page storage ceiling.
  The ceiling does not bound transient WAL size. Maintenance removes at most
  256 expired receipt records and 256 expired send records per call and commits
  separately so quota refusal does not undo cleanup progress.

Resource limits currently include 4,096 participants, 256 retained channels and
64 members per channel. Limits refuse new work; they do not evict history or
change recipient obligations. The caller must use trusted host time, validate
live grants and dispatch blocking database work off the editor loop.

Astra Medium reviewed the foundation until clean after fixes for durable
first-use directory creation, concurrent file creation, indexed receipt cleanup,
and expired-receipt renewal. Focused tests cover restart/replay, late subscribers,
all-recipient deletion, native policy fences, ten participants, bounded reads,
concurrent stores, expired records, disk-full rollback, bounded busy refusal and
unknown/corrupt/future databases. Linux formatting, core and desktop clippy,
core and desktop tests, and canonical coverage passed. Overall line coverage
is 92.14%; the new storage module has 96.02% line coverage. Windows execution
remains for CI. These checks verify the foundation, not the pending integration.

#### Opt-in live-trial integration

An explicitly configured host (`RUNYTE_CHANNEL_HOME`) advertises a separate
`runyte.context.channel.durable.v1` feature and serves typed `channel.durable`
requests on one bounded worker. With no override it advertises no durable
feature, opens no database and starts no worker. The MCP catalog adds
`durable_channel` without changing the legacy channel's behavior.

A pre-provisioned `--channel-profile` file binds a private recovery credential to
one workspace and configured store path. A retained kernel lock rejects concurrent
use of the same profile. The adapter never returns credentials to the model.
Profile installation is currently fixture-owned; general profile management is
not complete. No implicit recovery creates replacement identities.

The live path supports explicit registration/recovery, public subscription,
fixed groups, immutable send retry arguments, history pages, oldest-pending
reads and exact-batch acknowledgement. It requires current grants and negotiated
features, rechecks store policy, refuses queue overflow, and attaches replies to
the original cancellation lease. A transaction already admitted may commit
during revocation; its reply is suppressed and its original key resolves an
uncertain send after authorization is restored. Pending jobs with cancelled
leases are skipped. Public and fixed-group conversations remain local to the
granted workspace; native cross-workspace policy methods are not agent tools.

This trial does not replace the native channel page or terminal unread
indicators. Participant and channel IDs must be supplied explicitly. The
acceptance fixture and manual subagent driver are under `tests/mcp/`; all runtime
files belong to temporary directories outside tracked development context.

#### Live SQLite trial evidence

The isolated live trial used one persistent host, an alternate temporary SQLite
directory and three independent MCP adapters: coordinator, receiver and peer.
The two subagents drove the real `durable_channel` tool through their own stdio
adapters. The harness performed no scheduled reads or automatic acknowledgements.

Observed results:

- Public exchange and repeated reads returned the same pending batch and receipt
  until explicit acknowledgement. Public history remained readable afterwards.
- Receiver adapter restart and explicit recovery preserved its participant ID.
- A fixed group containing all three participants retained its message after the
  receiver acknowledged while the peer deliberately withheld acknowledgement.
- A separate two-member DM was readable by its recipient; the non-member peer's
  synthetic negative read was refused without returning a message body.
- Restarting the persistent host preserved public history, participant recovery,
  and the peer's exact outstanding group message and receipt.
- The peer's final acknowledgement deleted the fixed-group body; both subagents
  confirmed empty group history. SQLite inspection found no remaining DM/group
  bodies, thirteen retained public messages and `integrity_check = ok`.
- Retrying the original group send after deletion returned its original sequence
  without recreating the message.

Both adapters and the temporary host were closed and fixture storage removed.
The automated real-host test additionally checks twenty-five public messages,
latest-twenty subscription, explicit older-history paging, profile exclusivity,
strict tool fields, receipt replay and restart recovery. Focused Rust tests
cover direct host admission, cancelled worker jobs, private profiles and strict
protocol validation alongside the storage suite. Astra Medium reviewed the
integration repeatedly until clean; real-host testing also caught and fixed
workspace-ID length, strict empty-variant decoding and array-schema bounds.

Linux verification passed: formatting, core and desktop all-target Clippy,
the full core and desktop test suites, all 23 Unix MCP acceptance tests, and
canonical package coverage at 92.09% of lines (89% floor unchanged). Platform
execution on macOS and Windows remains for CI. The completed coverage target
and temporary Python environment were removed after recording results.

This was a prompted storage/integration test, not evidence of automatic skill
selection, independent native Claude Code participation, autonomous wake-up,
ten concurrent agents or desktop presentation. The bundled skill describes the
legacy in-memory tools; durable trial onboarding uses the separate explicit
workflow in the MCP guide. General skill/profile onboarding remains pending.

Still required before general Stage 3 deployment:

1. Graduate the opt-in host/MCP trial after integration and user-test coverage;
   preserve the separate legacy protocol and documented revocation boundary.
2. Explicit private adapter profiles, durable credential installation, concurrent
   profile-use rules and host-incarnation fencing for presence. Credentials
   must never reach model-visible results, channel text or logs.
3. A native operator policy-management surface for workspace/identity routes
   and participant recovery bindings, with revision-checked policy changes.
4. Conversation/participant discovery, the bounded human channel view and
   admission for at least ten live agents. The storage test with ten participants does not
   establish transport admission for ten agents.
5. End-to-end restart/revocation/mixed-client tests, desktop parity and updated
   skill/setup instructions before enabling the feature.


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

### 4. Channel nudges

Superseded on 2026-10-29: nudges, permission `7` and `:agent-bind` were removed
by the [read and write permissions plan](PLAN_MCP_READ_WRITE_PERMISSIONS.md).
The record below explains the earlier decision.

Decided 2026-10-26 after the manual user test above and tracked in
[#40](https://github.com/runyte/runyte/issues/40); implemented (see the
evidence below). It
applies to the legacy in-memory channel, since Stage 3 is deferred. Agents
still never send Enter. Instead, when a sender asks for it, the host may submit
one fixed line, a nudge, into a recipient agent's terminal to tell it to read
the channel. Message text never travels through the terminal; the agent reads
it with the channel tools, so a nudge adds no new path for peer-provided text
to reach a prompt.

Decided rules:

- The nudge text is a constant owned by Runyte, provisionally
  `Runyte: you have new messages on the agent channel.` Agents and channel
  messages cannot change it. It must be inert as a shell command: its first
  word resolves to no command on a case-insensitive filesystem and it contains
  no shell metacharacters. (A line starting with `Read` would run macOS's
  `/usr/bin/read` and leave the shell waiting for input.) Landing in a shell
  after the agent has exited therefore prints an error and nothing else.
- The text is typed as a bracketed paste followed by Enter, under the same
  bracketed-paste condition as direct insertion (`5`). A terminal without
  bracketed paste on is not nudged. The multiplexer caveat of `5` applies.
- Nothing nudges automatically. The sender decides per message whether a nudge
  is needed and whom it reaches, through an optional `nudge` argument to
  `post_message` listing participant nicknames or ids. For a public message it
  may name any current participants other than the sender; for a DM it may name
  only the recipient. Duplicates, unknown participants or the sender reject the
  whole post before it is stored, so a post never half succeeds. The list is
  bounded by the participant limit.
- The post's result reports each named recipient's nudge outcome: `sent`,
  `held` (see below), `already_pending`, or `unavailable`. `unavailable` covers
  every reason a nudge cannot be delivered, without telling the sender which.
  The message itself is stored regardless of nudge outcomes.
- The host submits the nudge; agents gain no tool for submitting terminal input.
  A nudge goes only to a participant that is currently connected and associated
  with a verified terminal, so Windows terminals, which have no verified
  association today, are not nudged.
- The person enables nudges through a new grant permission, provisionally `7`,
  given with physical input like every other grant. It requires `6`. Its
  description must state the remaining risk below. With one identity shared by
  several clients, the grant covers every terminal that identity's participants
  are verified in.
- At most one nudge per terminal is outstanding. Once a nudge is sent or held
  for a terminal, further requests for it report `already_pending` until its
  participant reads the channel. A read also discards a nudge still held for
  that participant, since it is no longer needed. Nudges never acknowledge
  messages or move read state.
- A nudge is held while the person may be typing in that terminal. The terminal
  records the time of its last physical input and whether that input left the
  line unsubmitted: any key or paste other than Enter marks a draft, as does a proposal's
  inserted text; Enter without Alt
  or Ctrl-C clears it. Hold the nudge while there is a draft, and until a quiet
  interval of 30 seconds has passed since the last physical input. Also hold it
  while a terminal proposal for that terminal is queued.
  Holding is the safe default: a held nudge waits, and the unread badge already
  shows that messages are waiting.

Checking the terminal's foreground process was considered and not adopted.
Grant identities are shared across clients (the test above used one identity for
Claude Code and Codex), so a process check cannot confirm the intended agent;
inert nudge text already makes the shell case harmless; and the check would be
Unix-only.

Remaining risk, accepted and stated in the grant: if the agent is showing an
approval dialog when a nudge arrives, Enter may choose its highlighted option,
which is often approval. Runyte does not inspect the screen for dialogs; a
screen heuristic would be wrong in both directions. The draft and quiet-interval
holds narrow this window but do not close it.

Implementation boundaries: nudge request validation belongs with the channel in
`src/workspace/context/channel.rs`; nudge policy and the outstanding-nudge state
belong to `src/workspace/host/context_channel.rs`; the physical-input record and
the paste write belong to `src/terminal/`; the `post_message` argument and result
belong to `src/mcp/`. The new authority must be added to
`context/reference/process-boundaries.md`, and the permission dialog change
follows the UI vocabulary and keymap references. `docs/mcp.md`, the user guide
and the bundled skill describe the argument; the skill should tell agents to
nudge only when the recipient must act and is likely waiting, and to rely on
ordinary reads otherwise. Changing the skill increments its `VERSION`.

Tests cover nudge validation (DM recipient only, sender, unknown and duplicate
names rejecting the post), each outcome, each hold condition, the
one-outstanding rule and its reset on read, the bracketed-paste refusal, grant
revocation while a nudge is held, and that a nudge never changes read state.

#### Implementation and trial evidence, 2026-10-26

The host types the line as a bracketed paste, waits 250 ms after it is written,
and then writes a single carriage return only if the terminal's input generation
is unchanged; otherwise the line is left as a draft and the nudge stays
outstanding. A paste cancelled by typing before it was written returns the nudge
to held. A held nudge sets a timer only for the end of its quiet interval, and
none while a draft, a proposal or paste mode holds it: the keystroke, decision
or terminal output that ends those holds already wakes the event loop.
A post's nudge advances only the nudge it adds, because the sender's own reader
is out of the host's table while its request runs. Unit and host tests cover
validation, every outcome, each hold, the stray-Enter guard, a sender keeping its
own pending nudge, revocation while held, and unchanged read state. One
independent review found two should-fix state-machine defects, an Alt-Enter
draft defect and test gaps; its second pass found that a nudge held past its
quiet interval by a draft gave the event loop a zero timeout. All were fixed,
and the review's third pass covered those fixes.

A sandboxed live trial used a real persistent host, an attached `--mux` frontend
on a PTY and three integrated terminals, each running a stand-in prompt that
reads pastes and owns its own `runyte mcp` adapter, so every participant's
terminal was verified. All runtime state lived in temporary directories. Two
subagents played a coordinator and an idle worker that woke only on prompt
input. Observed: every nudge reached the worker's prompt within about 0.1–0.6 s
and was followed by a read about 3 s later; request-to-reply rounds took 9–11 s;
a second nudge 33 ms after the first reported `already_pending` and produced one
line; an identity without `7` reported `unavailable` while its message was
stored; a nudge naming a non-recipient, the sender or an unknown participant
rejected the post without storing it; the worker's nudge back reached the
coordinator. Typing a draft in the worker's terminal held a nudge for over 40 s;
after Enter submitted the draft, the nudge arrived 31 s later.

The trial was repeated after the review fixes with the same results, rounds of
15–16 s, and refusal errors that name the participant and state that nothing
was posted. On the final build a nudge held by a draft for 45 s cost the
persistent host 2 clock ticks of CPU over 10 s, and arrived 31 s after the draft
was submitted.

This trial does not establish behavior of real Claude Code or Codex prompts,
including how they treat an Enter arriving while an approval prompt is shown.

Confirmed after the trial, 2026-10-27: nudges do not require an attached
frontend, unlike direct insertion, so agents can wake each other while the
persistent session is detached; the holds and the grant still apply. A
remembered grant that includes `7` cannot be read by earlier releases, which
then load none of that workspace's remembered grants, as already happens with
`6`; the user guide states this and no compatibility shim is planned.

Lifecycle hooks (SessionStart, tool-boundary, Stop) remain a tentative
complement for reminding agents while they work. They are optional, separately
installed and must not acknowledge messages. They are not needed for the idle
case nudges address.

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
