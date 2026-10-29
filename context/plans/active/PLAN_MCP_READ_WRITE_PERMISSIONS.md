# MCP read and write permissions

Status: active on `dev`, 2026-10-29. Related:
[#43](https://github.com/runyte/runyte/issues/43),
[#42](https://github.com/runyte/runyte/issues/42) and
[#40](https://github.com/runyte/runyte/issues/40).

## Problem

Agent access to a workspace had grown into seven numbered permissions
(terminal read, editor read, buffer edit, terminal proposals, direct
insertion, channel and channel nudges). Waking an idle agent needed one of
three mechanisms, none of them dependable:

- Terminal nudges typed a fixed line and Enter into a terminal Runyte had
  to associate with the participant. A Codex daemon's adapter has no
  terminal Runyte can verify, so `:agent-bind <participant> <number>` was
  added, and its arguments are hard to discover. The Enter could answer a
  pending dialog the nudge could not see.
- Claude channel notifications depend on a research-preview client
  extension, a development flag and account eligibility.
- Codex queue notifications depend on an undocumented, version-tested CLI
  command and a hand-configured thread UUID per adapter.

The notification experiment stays on `exp`, recorded there in
`context/plans/active/PLAN_CHANNEL_NOTIFICATIONS.md`. `dev` reverts it
(`6493093`) and replaces the whole wake-up problem with a model where an
agent can see what the person sees.

## Decisions

### Two permissions

Each MCP identity has two permissions per exact workspace, toggled in
`:mcp <identity>` and applicable at any time while agents run:

| Key | Permission | Grants |
| --- | --- | --- |
| `1` | Read | Every buffer, pane, selection and viewport; every terminal's screen, scrollback, number and title; the agent channel, including direct messages and posting |
| `2` | Write | Editing and appending to buffers, typing into terminals and pressing Enter there; requires Read |
| `r` | Remember | Keep this exact workspace's grant across hosts |

- Both are off by default. Nothing is granted until the person applies a
  grant. `mcp.default_grants` remains an opt-in per-user setting and can name
  only `read`. The capability names earlier releases accepted there
  (`terminal_read`, `editor_context_read`, `agent_channel`, `buffer_edit` and
  `terminal_propose`) are still accepted rather than failing the
  configuration; a list with both `terminal_read` and `editor_context_read`
  means Read, and any other grants nothing and is reported at start. `write`,
  and the names earlier releases already refused, are rejected: Write is
  granted per workspace only.
- Channel participation belongs to Read, so agents can coordinate without
  being able to write anywhere.
- Turning Write on turns Read on; turning Read off turns Write off.
- Remember is available for Write. The dialog starts with Remember as it is
  stored, so a session grant stays a session grant.
- Applying a changed grant disconnects that identity's readers, so the new
  grant takes effect at once; their adapters reconnect on the next call.
  Applying a grant no longer retires and restarts the context server when
  the identity was the only one granted, so workspace handles stay stable
  across a grant change. Revoking the last grant still retires it.

### Wire representation

The context profile keeps capability names as the unit of negotiation, so
clients still declare what they use. A grant maps to capabilities:

- Read: `terminal_read`, `editor_context_read`, `agent_channel`.
- Write: Read plus `buffer_edit` and `terminal_write`.

`terminal_propose`, `terminal_insert` and `channel_nudge` are removed. A
registration naming `terminal_propose` as an optional capability (an older
client's request) is accepted and never granted, so an older adapter keeps its
read access rather than failing registration. One that requires it is refused,
since it cannot work. A host offering terminal writes lists
the feature `runyte.context.terminal.write.v1`; a client asks for
`terminal_write` only after seeing it.

Remembered grants are stored as `permissions: ["read"]` or
`["read","write"]`. A record in the older `scopes` form is read as Read only
when it held both `terminal_read` and `editor_context_read`, and as a
revocation when it held less (Read is wider than what such a grant allowed,
and the person's narrower choice still keeps a configured default off) or was
empty. It is never read as Write. Configured defaults
follow the same rule: legacy names that do not include both reads grant
nothing, and the host reports that identity at start. An older release cannot read the new form and then loads no
remembered grants for that workspace.

### Terminal writes

`terminal.input.write {terminal, text}` requires `terminal_write`, a live
terminal and an attached frontend, so the person can see it happen. The
text is one line of at most 4 KiB with no controls, as before; Enter is a
separate request. It is queued behind any other pending input and sent as a
bracketed paste when the child enabled bracketed paste, or as typed
characters otherwise. The reply is `queued`; a lost acknowledgement is
`outcome_unknown` and is never retried. Queued text is cancelled when its
reader disconnects (including for a malformed request), when the person
types there, and when the frontend it was typed under detaches or changes.

### Enter

`terminal.input.enter {terminal, expected_revision}` asks for one Enter.
The revision is the terminal's read revision from a `terminal.read` taken
after the text appeared. Runyte:

1. refuses with `stale` if the terminal's read revision already differs;
2. records the input generation and shows `[Enter pending]` in the pane
   title;
3. after five seconds, arms it, and on an event-loop pass at least 100 ms
   later, after any output or input already queued has been handled, sends a
   carriage return only if the read revision and input generation are both
   unchanged, the reader still holds Write, the frontend is attached and the
   terminal is live.

The read revision changes with input as well as output, so text typed into
a program that echoes nothing still invalidates a revision an agent read.

Any output (a dialog appearing, a spinner, the echo of late text), any
physical key or paste, and any other agent's write therefore make it `stale`.
Detaching and terminal exit make it `cancelled`. A grant change, revocation
or disconnect closes the reader and discards its Enters with it; the MCP
`press_enter` then reports `outcome_unknown`, because the Enter may have been
pressed just before. One Enter
may wait per terminal; four per reader and sixteen per host.

The request answers at once with an `enter` handle and state `waiting`.
`terminal.input.enter.status {enter}` reports `waiting`, `sent`, `stale` or
`cancelled`; finished records are kept for two minutes, at most 64. The MCP
`press_enter` tool waits out the delay and returns the final state, so an
agent makes one call.

This closes the gap a delay alone leaves: an agent that read the screen and
saw no dialog cannot have its Enter land on one that appeared afterwards,
because the dialog's output changes the revision. A program that redraws
constantly never receives an Enter this way, which fails safe.

Write grants are powerful: an agent can run commands in a shell or answer
another agent's approval prompt. The grant dialog says so, and the person is
expected to grant Write for the duration of a job and revoke it afterwards.

### Inter-agent communication

- The channel (posts, direct messages, reads) needs Read.
- An agent that must wake another reads the terminals (titles show
  `[terminal #N]`), writes a line into the recipient's terminal and presses
  Enter, which needs Write.
- Terminal association stays automatic where Runyte can verify it (an
  integrated terminal on Unix), for the `[N unread]` title marker and the
  participant's `terminal` field. `:agent-bind` and native bindings are
  removed; nothing replaces them.
- Channel nudges (`nudge` on `channel.post`, feature
  `runyte.context.channel.nudge.v1`) are removed.

### Reviewed proposals

`terminal.input.propose/status/cancel`, the native proposal review overlay
and its `ContextReview` frame layout are removed. Under this model text an
agent sends is direct and visible, and Enter has its own guard. The bundled
protocol version moves to 82 because the layout case disappears.

### MCP adapter

Tools: `find_resources`, `list_workspaces`, `read_buffer`, `read_terminal`,
`append_buffer`, `edit_buffer`, `write_terminal`, `press_enter`,
`join_channel`, `post_message`, `read_messages`, `list_participants`, and the
experimental `durable_channel`: thirteen. `propose_terminal_text`,
`terminal_proposal_status` and `cancel_terminal_proposal` are removed.

`read_terminal` defaults to `screen`. `tail` adds scrollback and is for when
a task needs history. `list_workspaces` reports `permissions` (`read`,
`write`) beside the negotiated capability names.

The skill tells agents to read a terminal's screen before writing to it,
and again before pressing Enter, to use scrollback only when a task needs
it, and to wake another agent by writing to its terminal only when it must
act and is idle at its prompt.

### Internal naming

`terminal::proposal::{Text, Delivery}` keep their names; their comments now
describe agent text. Renaming the module would touch the PTY writers on both
platforms for no behavior change.

## Implementation sequence

1. Revert the notification commit on `dev` (done, `6493093`).
2. Wire and storage: capabilities, requests, feature, record migration,
   configuration validation.
3. Host: grant model and dialog, writes, Enter scheduling and title marker,
   removal of proposals, nudges and bindings, grant changes without server
   retirement.
4. Frontend: remove the proposal review overlay, its layout and
   `:agent-bind`; protocol version 82.
5. MCP adapter: tools, negotiation, `press_enter` waiting, defaults.
6. Documentation: MCP guide, context contract and schema, Python client,
   user guide chapters, configuration, process boundaries, UI vocabulary,
   keymap register, skill, agent guide.
7. Tests: unit and host tests at each boundary, MCP acceptance tests, and
   the coverage floor.

## Verification

Completed on Linux on 2026-10-29, on top of `6493093`:

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and the
  desktop crates' Clippy with warnings denied: pass.
- `cargo test`: 4,971 passed. Five earlier full runs were clean; one run, and
  one coverage run, hit
  `cli::skills::tests::live_lock_refuses_another_installer_and_releases_on_close`,
  which passes alone. Its `flock` descriptor can be held briefly by a child
  that another test forks at the same moment; the skills installer is
  unchanged by this work.
- `cargo test -p runyte-native -p runyte-desktop -p runyte-preview`: 129
  passed, three ignored as before.
- `cargo llvm-cov --locked --package runyte`: 92.20% of lines, above the
  unchanged 89% floor; `context_input.rs` 91.98%.
- `tests/mcp/run.py`: 23 passed, including a write and guarded Enter through
  `runyte mcp`. `docs/plugins/check_context.py`: 15 passed.
- Four subagent reviews: host security, documentation against code, tests
  and Windows code, and a final pass over the fixes. Their findings were
  fixed: queued text outliving a reader closed for a malformed request or a
  detached frontend, an Enter pressed before already-queued output was
  handled, the revision ignoring unechoed input, legacy grants widening to
  Read, a required retired capability accepted silently, and documentation of
  outcomes agents could not receive.

Not verified here: Windows and macOS builds and tests. The Windows-only code
and tests were updated by reading against the current APIs and reviewed the
same way; CI must establish them.
