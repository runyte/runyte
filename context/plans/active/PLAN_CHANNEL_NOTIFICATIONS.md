# Terminal-free channel notifications

Status: experimental implementation on `exp`, 2026-10-28. Tracking:
[#43](https://github.com/runyte/runyte/issues/43), related
[#40](https://github.com/runyte/runyte/issues/40) and
[#42](https://github.com/runyte/runyte/issues/42).

## Scope and integration

`dev` was fast-forwarded into the clean `exp` worktree first, from `6fc3769`
to `8165a2e`. This experiment remains local to `exp`: no push, GitHub mutation
or merge back into `dev`. Durable SQLite channel work remains deferred.

The objective is waking a participant without terminal input, preserving
pending choice and approval dialogs. Legacy terminal nudges remain an explicit
backup. A failed notification must never automatically select that backup.

## Sources and compatibility decision

Official documentation consulted on 2026-10-28:

- [Claude channels reference](https://code.claude.com/docs/en/channels-reference):
  server capability `experimental.claude/channel`, notification method
  `notifications/claude/channel`, string `content` and string-valued `meta`.
- [Claude channels](https://code.claude.com/docs/en/channels): research-preview
  eligibility, organization policy and development-channel enablement.
- [Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli): MCP setup,
  but no documented channel notification that starts a model turn.
- [Codex App Server](https://learn.chatgpt.com/docs/app-server): thread, turn and
  approval lifecycle; the installed queue command is not documented there.

There is no established portable MCP wake that covers both clients. Claude
uses its documented extension. Codex uses its installed `queue` command,
verified with CLI 0.164.0. This is an experimental client-specific sidechannel,
not a claim that Codex handles Claude notifications or standard MCP logging as
new prompts. Other clients remain pull-based.

Claude requires explicit development-channel enablement and client eligibility.
The adapter never advertises `claude/channel/permission` and cannot answer tool
approval requests. Codex requires an exact persisted thread UUID in trusted
adapter launch configuration and the matching daemon environment. No thread is
inferred from `provider`, `clientInfo`, a tool argument or inherited daemon
thread metadata. A thread-bound configuration must not be reused across agents.
This manual setup is deliberate: daemon-spawned MCP adapters cannot reliably
infer the active terminal or thread.

## Protocol and authority decisions

- Negotiate `runyte.context.channel.notify.v1` independently of legacy nudge
  support. Only an explicitly configured recipient adapter subscribes, through
  `notifications: true` in `channel.join`; this is not a public MCP tool field.
- Permission `7` (`channel_nudge`) still belongs to the recipient and requires
  `6`. No global default or extra grant is introduced. Its disclosure continues
  to describe the risk of the explicit terminal backup.
- New MCP `post_message` defaults to `nudge_transport: notification`. A sender
  can explicitly choose `terminal`. Old context clients retain their omitted
  transport semantics; an updated adapter never silently downgrades an older
  host to terminal input.
- Host events contain only `{"type":"channel_notification"}`. The adapter
  supplies a fixed reminder and its own workspace handle. Peer message text,
  task descriptions, nicknames and thread IDs do not enter delivery arguments.
- `queued` means host acceptance, not confirmed model processing. Client
  extension gating can discard events. Codex subprocess timeout may represent
  an uncertain delivery; there is no retry. Messages remain readable.
- One outstanding notification per connection until successful channel read;
  no terminal binding is needed. Revoke, regrant and disconnect invalidate the
  old connection. Already emitted notifications cannot be recalled.

## Resource and lifecycle decisions

The existing authenticated context socket/pipe carries unsolicited negotiated
frames. A lease retains one pending bit and one outstanding bit. Notification
subscribers remain connected while idle beyond the ordinary five-minute
request-reader expiry, without adding a polling timer. EOF and revocation still
close them; writes retain a two-second deadline. Unix and Windows use the same
state rules.

Pinned read futures preserve partial frames when a notification arrives. The
MCP connection driver separates replies from notifications with bounded queues;
its lifetime flag suppresses stale buffered events. The adapter services events
while waiting for stdin and while processing a tool request. Codex delivery
uses a separate bounded worker so process startup cannot suspend tool calls.
There are at most eight queued Codex deliveries and one child, with a five-second
child deadline, null stdio, argument-vector launch and kill-on-drop. Delivery
failures remain local diagnostics and do not cause terminal writes.

## Reviews and validation

Astra Medium reviewed the initial design and implementation. The first code
review found three defects: mismatched feature negotiation ordering, missing
workspace routing in Codex reminders, and inline Codex delivery suspending MCP
operations. All three were corrected before acceptance testing. Two further
Astra Medium passes found no remaining code issues; a stale
user-guide description was corrected. Actual Unix and Windows transport tests
were added for fragmented input across notifications and revoke-before-write.
Final validation results are recorded below.

Sol sandbox probes used fixture-owned configuration and local fake model
services, without real API credentials. Codex 0.164.0 accepted a wake through the
actual default managed-daemon `codex queue` path, started an idle persisted
thread, and retained another wake behind a real command approval without
answering it. The retained probe is `tests/mcp/codex_queue_probe.py`.

Claude 2.1.312's fresh fake-key account did not have channel rollout eligibility.
A fixture-only cached feature override allowed testing development-channel
consent, notification receipt and automatic model dispatch. This establishes
client protocol behavior under simulated eligibility; it does not establish
availability for an account or organization. The retained probe is
`tests/mcp/claude_channel_probe.py`. With the installed
Claude 2.1.312, the real Runyte host and MCP adapter, and no terminal binding, a
sender's wake started model dispatch and a real `read_messages` call. A second
wake while `AskUserQuestion` was open left it unanswered: no model requests and
zero terminal input were observed during the pending-choice window. The fake
model controls the tool sequence; this is not a test of model judgment.

The Codex probe also passed through the real Runyte host and recipient MCP
adapter, not only through the standalone CLI command. It checked the exact
workspace handle in the wake, absence of peer text, idle dispatch and queued
input behind a pending command approval after the recipient had read the
channel. Both vendor scripts accept the built Runyte binary path.

The five mandatory notification acceptance cases passed against real Runyte
processes: idle wake with no terminal, recipient permission enforcement,
coalescing/read reset, delivery during another workspace's long read,
revocation and unsupported-recipient behavior. They run in the ordinary MCP
acceptance runner. Vendor probes are optional because installed vendor clients
and local socket permissions are external prerequisites.

A first full Rust run found one new transport-test assumption: closing a socket
with unread partial input can return `ConnectionReset`, not only EOF. The
Unix/Windows regressions now accept EOF/reset/broken pipe while independently
asserting that no bytes were published. No production behavior was weakened.

Native macOS and Windows execution and their coverage cannot be established on
this Linux machine. The Windows transport implementation and matching tests
were reviewed, but this experiment has not been pushed to CI. A five-minute
wall-clock idle test was not added; subscribed readers explicitly use no idle
timeout, while revocation and EOF remain active.

## Final Linux verification

Completed on 2026-10-28 after the fixes above:

- `cargo fmt --check`: pass.
- `cargo clippy --all-targets -- -D warnings`: pass.
- `cargo test`: pass, including the new notification and transport regressions.
- `cargo clippy -p runyte-native -p runyte-desktop -p runyte-preview --all-targets -- -D warnings`: pass.
- `cargo test -p runyte-native -p runyte-desktop -p runyte-preview`: pass
  (129 passed, three existing optional tests ignored).
- `cargo llvm-cov --locked --package runyte`: pass; total Lines **92.10%**,
  above the unchanged 89% floor. Used the existing local cargo-llvm-cov tool;
  no coverage threshold or instrumentation exclusions were changed.
- Full `tests/mcp/run.py`: **28 passed**, no skips, including the five new cases.
- `tests/mcp/codex_queue_probe.py` with the built Runyte binary: pass on
  installed Codex 0.164.0, real daemon and real Runyte, fake local model.
- `tests/mcp/claude_channel_probe.py` with the built Runyte binary: pass on
  installed Claude Code 2.1.312, real Runyte, fake local model and simulated
  preview eligibility; pending choice preserved without terminal input.
- Final Astra Medium review: no remaining findings. Native macOS/Windows
  execution and actual account-specific Claude eligibility remain unverified.

The implementation and retained probes are experimental. The existing terminal
backup still has the documented Enter-at-dialog risk and remains explicitly
selectable. This local work does not close #43 or change GitHub status.
