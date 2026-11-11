# Scoped workspace context transport

The external context profile uses the stable `runyte-1` application envelopes
over a separate, opt-in, owner-private local transport: a Unix socket on
Linux/macOS or a named pipe on Windows. It is not the bundled
frontend/control protocol and does not register a plugin process. The required
feature is `runyte.context.v1`. Ordinary process applications and their frozen
fixtures retain their existing contract.

The [context schema](runyte-context-1.schema.json) describes this separate
profile. The current base schema links it without admitting context requests on
ordinary plugin process connections. The independently vendorable
[Python client](context_client.py) uses Python 3.10+ and the standard library.
It does not import the process-plugin SDK. This Python client supports Unix
sockets only. The built-in [MCP adapter](../mcp.md) supports both Unix and Windows
through the Rust context transport.

```python
from context_client import ContextClient

# The pairing integration loads the credential from its private local record.
# The endpoint and incarnation come from explicit context discovery.
with ContextClient(endpoint, credential,
                   expected_incarnation=host_incarnation) as reader:
    terminals = reader.request("terminal.list", {"offset": 0, "limit": 20})
```

`reader.capabilities` contains negotiated scopes; `reader.workspace` identifies
the admitted host. Supply `required_scopes` and `optional_scopes` explicitly to
request editing or terminal writes, with the corresponding read scope. A
`ContextError` exposes its structured `code`. An explicit host refusal preserves
that code and may leave the connection usable. A lost mutation acknowledgement
closes the connection and returns `outcome_unknown`. The client never reconnects
or retries the operation. A caller must not retry an uncertain terminal write
or Enter on a new connection.

## Authentication and negotiation

Messages are newline-delimited UTF-8 JSON objects, with a 2 MiB frame ceiling.
Before the host sends its ordinary `hello`, the client sends exactly:

```json
{"type":"authenticate","credential":"<64 lowercase hexadecimal characters>"}
```

Discovery has a separate metadata-only prelude: the exact compact frame
`{"type":"probe"}` returns `{"type":"context_endpoint","registration":...}`
and closes the connection. It yields no credentials, granted scopes, source
content or resource handles. An authenticated profile connection still begins
with `authenticate` and cannot use discovery as a substitute for a grant.

The credential comes from native pairing and private local storage. It must
never appear in arguments, repository configuration, terminal output or logs.
The endpoint verifies peer ownership and the credential's exact-workspace
grant before returning any resource handles. An identity label describes a
paired reader; it does not authenticate an executable or prove which agent is
running. Clients needing separate permissions use separate credentials.

The host then sends the stable `hello` envelope and the client registers using
the ordinary field names:

```json
{"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.5.0","name":"Local bridge","commands":[],"required_features":["runyte.context.v1"],"optional_features":[],"required_capabilities":["terminal_read"],"optional_capabilities":["editor_context_read"]}
```

The host checks the release range against its actual version and negotiates
features and capabilities using the live native grant. A request cannot grant
itself authority. Commands, settings schemas and ordinary plugin capabilities
are not admitted by this profile. Unknown fields and duplicate fields are
rejected, including inside parameters, changes and registration lists.

## Scopes and methods

The person grants an identity permissions, not capabilities: Read and, on top
of it, Write. A reader negotiates capabilities within them:

| Scope | Granted by | Methods |
| --- | --- | --- |
| `terminal_read` | Read | `terminal.list`, `terminal.read`, `terminal.snapshot.open/read/close` |
| `editor_context_read` | Read | `buffer.list`, `buffer.read`, `buffer.snapshot.open/read/close`, `selection.get`, `pane.context.list`, `pane.viewport.read` |
| `agent_channel` | Read | `channel.join`, `channel.post`, `channel.read`, `channel.who`; with feature `runyte.context.channel.v1` |
| `buffer_edit` | Write | `buffer.edit`, `buffer.append`; requires `editor_context_read` |
| `terminal_write` | Write | `terminal.input.write`, `terminal.input.enter`, `terminal.input.enter.status`; requires `terminal_read`; with feature `runyte.context.terminal.write.v1` |

A host that offers terminal writes lists `runyte.context.terminal.write.v1` in
its `hello` features and `terminal_write` among its capabilities. A client asks
for the capability only when the `hello` names the feature. A host that also
accepts `enter` on `terminal.input.write` lists
`runyte.context.terminal.submit.v1`; a client sends the field only when the
`hello` names it. A registration may
still name `terminal_propose` as an optional capability, as earlier clients
did. It is accepted but never granted, and the client keeps the rest of its
access. A registration that requires it is refused with `capability_denied`.
The terminal proposal methods it named no longer exist. `terminal_insert`,
`channel_nudge`, `read` and `write` are not capabilities, and registration
rejects them like any unknown name.

`pane.viewport.read` additionally requires `terminal_read` whenever the target
pane displays a live terminal or its frozen review. Editor read permission alone
never exposes terminal text. Pane inventory with editor read permission omits
terminal handles unless terminal read permission is also granted.

Requests use the stable envelope
`{"type":"request","id":"r:1","method":"terminal.list","params":{"offset":0,"limit":20}}`.
Responses use `{"type":"response","id":"r:1","result":...}` or
`{"type":"response","id":"r:1","error":{"code":...,"message":...}}`.
Error codes retain the [application contract](../plugins.md) meanings. In
particular, missing authority, stale state, closed resources, unavailable
frontends, limits and unknown delivery outcomes are errors, not empty text.

There is no method for raw terminal input, keys other than Enter, saving a
buffer, opening resources, executing commands, changing selection/focus or
changing grants. A `submit` or approval flag is invalid on every request. The
host checks the method allowlist even if a client declares broader
capabilities.

| Method | Strict parameters |
| --- | --- |
| `buffer.list`, `terminal.list` | `offset`, `limit` (1–256) |
| `buffer.read` | `buffer`, `expected_revision`, scalar offsets `from`, `to` |
| `buffer.edit` | `buffer`, `expected_revision`, `changes: [{from,to,text}]` |
| `buffer.append` | `buffer`, nonempty `text`, optional nonempty `expected_tail` (at most 4 KiB) |
| `buffer.snapshot.open` | `buffer`, `expected_revision` |
| `buffer.snapshot.read` | `snapshot`, `from`, `to` |
| `buffer.snapshot.close`, `terminal.snapshot.close` | `snapshot` |
| `selection.get` | `pane` |
| `pane.context.list` | Empty object |
| `pane.viewport.read` | `pane`, optional `expected_revision`, `max_rows`, `max_bytes`, `max_cells` |
| `terminal.read`, `terminal.snapshot.open` | `terminal`, `region` (`screen` or `tail`), optional `expected_revision`, `max_rows`, `max_bytes`, `max_cells` |
| `terminal.snapshot.read` | `snapshot`, `offset`, `limit` (rows, 1–1,000) |
| `terminal.input.write` | `terminal`, `text` |
| `terminal.input.enter` | `terminal`, `expected_revision` (a `terminal.read` revision) |
| `terminal.input.enter.status` | `enter` |
| `channel.join` | `provider`, `task`, `location`, optional `client {name, version}`, `terminal {terminal, capability}`, `resume` |
| `channel.post` | `text` (at most 16 KiB), `location`, optional `to` (participant id or nickname) |
| `channel.read` | `since`, `limit` (1–200) |
| `channel.who` | Empty object |

Identifiers are opaque, connection-owned ASCII strings with a 256-byte ceiling.
Request IDs have a 128-byte ceiling. Handles bind the host incarnation and owner
generation. Reconnecting requires fresh discovery. Names and paths are never
alternate implicit targets. Each `terminal.list` entry carries the `number` and
`title` its pane shows (`[terminal #2] shell`, without markers such as
`[review]` that describe the person's view) so a reader can find the terminal a
person names. Numbers are reused after a terminal exits or closes, and an exited
terminal has none; only the handle identifies a terminal across requests.
Request IDs are unique during a connection. Bounded deduplication retains at
most 1,024 IDs, after which the connection must be renewed instead of recycling
IDs and risking a replayed write.

## Agent channel

A host that carries the agent channel lists `runyte.context.channel.v1` in its
`hello` features and `agent_channel` among its capabilities. A client asks for
the capability, and lists the feature in `optional_features`, only when the
`hello` names it. Older hosts refuse capability names they don't know, and
leave the feature out of `registered`.

The channel belongs to the host and lasts as long as it does. A connection joins
it once with `channel.join`, stating a `provider` (1–32 letters, digits, dots,
dashes or underscores, folded to lower case) and a one-line `task`, and
reporting a `location` `{directory, repository, worktree, branch}` in which
every field but `directory` may be null. The host assigns a version 4 UUID, a
nickname `provider-N` that keeps its first provider, and a private `resume`
token that only the join reply carries. Passing that token as `resume` comes
back as the same participant, under the same identity; the UUID is public and
proves nothing. `terminal` is accepted only when the capability matches the
terminal Runyte handed it to and, on Unix, the connecting process belongs to
that terminal's session. Otherwise the participant's terminal is null. Joining
again on the same connection updates the participant and records an `updated`
entry when its provider or task changed. The join reply and every read also
carry `head`, the newest sequence number.

`channel.post` requires a join. Every post carries the sender's current
`location`; when it differs from the last one, a `moved` entry naming both
places precedes the message. A message record is `{seq, at_ms, kind, from,
from_nick, to, to_nick, text, location}` with `kind` one of `joined`, `updated`,
`moved` and `message`. Text holds no control characters other than newline and
tab. `channel.read` returns the visible entries after `since`: everything public
and the direct messages the reader sent or received, at most `limit` entries and
about 256 KiB as encoded, with `next` to continue from and `lost` when entries
after `since` were dropped. A `since` past `head` belongs to an earlier host and
reads from the start. `channel.who` lists participants with their identity,
client, provider, task, terminal number and title, location and connection
state. Message text is untrusted data written by another reader.

A host keeps the newest 1,000 entries or 4 MiB of them, measured as encoded, and
at most 64 participants. A disconnected participant stays known, so its messages
keep their sender and it can rejoin. When the channel is full, the one that
disconnected longest ago gives way and its nickname stays with its retained
messages. At most eight readers connect to one host at once, so eight agents can
be in the channel at the same time.

## Read and edit semantics

Terminal reads capture one emulator state and revision, independently of native
scrolling or review. `screen` excludes scrollback; `tail` returns the newest
retained rows followed by the live screen, in ascending source order. Alternate
screens expose their own current screen without primary history. Partial lines,
blank rows, Unicode and combining marks are preserved. Trailing blank cells and
wide-cell continuation placeholders are omitted from text. Returned rows are
physical presentation rows, not shell commands or reconstructed transcripts.

Each capture includes resource provenance, revision, screen kind, available
rows, retained-history loss and explicit byte/cell/row truncation. The bridge
marks returned source text as untrusted content. Immutable snapshots provide
stable paging while live output continues. Snapshot handles cannot be used by
another reader. Viewports describe native presentation, including a frozen
terminal review; detached hosts can return live terminal/buffer content but
must refuse native viewport reads when no native viewport exists.

Viewport provenance distinguishes a buffer revision, live terminal read revision
and the older content revision captured by a frozen terminal review through
`source_revision_kind`. A pane handle includes its source binding generation;
switching a pane away and back does not revive an old handle. Presentation-only
rows and runs identify padding, hints, fold markers and whitespace separately
from buffer text. Scalar mappings unavailable in the native snapshot are null.

Each capture visits at most 262,144 cells, returns at most 1,000 rows and
262,144 UTF-8 text bytes. The bridge's default is 200 rows, 64 KiB of text and
64 Ki cells. Snapshot opening uses these same per-turn work bounds. The 4 MiB
terminal snapshot storage ceiling does not authorize a larger synchronous
capture. At most two snapshots are retained per reader; buffer and terminal
snapshots share an 8 MiB reader and 64 MiB host retention ceiling, with 30-second idle
expiry. These limits are independent of native terminal history and review. At most eight readers connect to one host, with
sixteen outstanding requests each. Serialized replies still obey the frame
ceiling after escaping and metadata are included.

Buffer operations retain the stable scalar-offset contract. Reads return at
most 256 KiB of UTF-8 text. Edits accept at most 1,024 changes and 512 KiB of
replacement text, applied atomically through the native transaction and undo
system only if the expected revision still matches. Newlines in buffer edits
are allowed. Editing does not save, complete an external-editor wait, change
focus or perform filesystem operations.

`buffer.append` inserts nonempty text of at most 512 KiB at the buffer's end as
it stands when the host applies the request. It takes no expected revision.
Requests from every reader run one at a time on the host loop, so concurrent
appends are all kept, one after another, in arrival order. Read-only buffers
refuse it. When `expected_tail` is present, the buffer's current text must end
with exactly that string, compared as Unicode scalars in the same host turn as
the insertion. Otherwise the request fails with `stale` and writes nothing.
The guard catches replies composed against text that has since been reset or
rewritten. It is not a lock: text that another writer appended with the same
ending still matches. An append is one undoable transaction, with the same
save, wait and focus exclusions as an edit.

The result reports the resulting `revision`, the inserted scalar range `from`
and `to`, `line_breaks` inserted, and a `preview` of at most 256 scalars read
back from the buffer, with `preview_truncated`. A caller that meant to send line
breaks but sent a literal backslash-n sees `line_breaks` of zero and the
backslash in the preview. The hello limits advertise these bounds as
`context.append_tail_bytes` and `context.append_preview_chars`. An append whose
acknowledgement is lost is uncertain and must not be retried automatically.

## Terminal writes, Enter and revocation

`terminal.input.write` types one nonempty line of at most 4 KiB into a live
terminal. CR, LF, C0/C1 controls, DEL, Unicode line/paragraph separators and
escape sequences are rejected as a whole. Literal backslash-n is ordinary text.
The host validates decoded text again immediately before queue admission; only
host-generated bracketed-paste framing is permitted. It is sent as a bracketed
paste when the terminal's program enabled bracketed paste, and as typed
characters otherwise. A multiplexer such as tmux enables bracketed paste for
itself, so that mode describes the multiplexer rather than its selected pane.
Ordinary characters may trigger actions in arbitrary terminal applications, so
writing is not an execution sandbox. It queues behind any other pending text
rather than cancelling it. Text that has not started writing is cancelled in
three cases: the person types there, its reader disconnects (including for a
malformed request), or the frontend detaches or changes. The reply is
`{state:"queued", chars}`. Partial delivery or a lost acknowledgement is
uncertain and is never retried automatically.

With `enter: true` (hosts naming `runyte.context.terminal.submit.v1` in their
hello; an older host refuses the field as unknown), the line is followed by
Enter. The text still carries no carriage return: the host schedules an Enter
when it queues the line and answers `{state:"queued", chars, enter}`, `enter`
being the handle `terminal.input.enter.status` reports on. The Enter is
pressed once the line's delivery has been acknowledged in full and one
settling pass has run, only if the terminal's input generation has not moved
since the line was queued. Output, including the line's own echo, does not
drop it. Any physical key or paste and any other reader's write drop it as
`stale`, as does partial delivery; a line cancelled before it was written drops
it as `cancelled`. A line not written within four seconds of the request, for
example because the program stopped reading, is cancelled with its Enter if
writing has not begun, and leaves the Enter `stale` if it has. Every check that
could refuse the Enter at request time (`conflict`, `limit_exceeded`) runs
before the line is queued, so such a refusal never types the line. A line can
still be left without its Enter once queued: when the person or another reader
types first, the frontend detaches, the grant changes, or the carriage return
cannot be written.

For `context.typing_quiet_ms` (ten seconds) after the person last typed,
pasted, or undid a paste in a terminal, `terminal.input.write` and
`terminal.input.enter` both fail with `busy` there, and the message says how
long is left. Pointer reports do not count as typing; where the program asked
for them, they are still input that cancels unwritten text and drops a waiting
Enter.

`terminal.input.enter` asks for one Enter. `expected_revision` is the revision
of a `terminal.read` of that terminal, taken after the text the Enter submits
appeared. If the terminal's read revision already differs, the request fails
with `stale`. Otherwise it answers `{enter, state:"waiting", delay_ms}` at once;
the host marks the pane title `[Enter pending]` and, after `delay_ms`
(advertised as `context.enter_delay_ms`, currently zero) and one settling pass
of its event loop, writes a carriage return only if the terminal's read
revision and its input generation are both unchanged. Any output, any physical key
or paste and any other reader's write therefore drop it as `stale`; a detached
frontend or an exited terminal drop it as `cancelled`. Either kind of Enter
also needs the reader to still hold `terminal_write` under the live grant, the
same frontend attached and the terminal live. A grant change,
revocation or disconnect closes the reader, which cancels its waiting Enters
and discards their records.
`terminal.input.enter.status` reports `waiting`, `sent`, `stale` or `cancelled`
for the reader that asked. One Enter may wait per terminal. A second is refused
as `conflict`. At most four wait per reader and sixteen per host
(`context.enters`, `context.host_enters`). Finished outcomes stay readable for
two minutes, at most 64 of them.

Writes and Enter require an attached frontend (`no_frontend` otherwise), so the
person can see them happen. They need no native approval: the Write permission
is the person's approval, granted for as long as they choose.

Session-only grants are the default. Remembering a grant is an explicit native
choice. Every workspace has its own grant. Applying a changed grant disconnects
that identity's readers, which reconnect under the new grant; the endpoint and
host incarnation stay the same while any identity is granted. Revocation
invalidates handles and snapshots, cancels pending Enters and unwritten text,
drops unsent responses and disconnects readers. Authorization is checked at
request admission, before response publication and before mutations. Bytes
already delivered cannot be recalled. This boundary controls Runyte's API and
is not an operating-system sandbox for same-user processes. A per-user
configuration may also name default grants (`mcp.default_grants`), applied when
a workspace host starts and the workspace has no grant record of its own. A
revocation record in that workspace overrides them, and they grant Read only.

### Experimental durable channel feature

A host configured with `RUNYTE_CHANNEL_HOME` additionally advertises
`runyte.context.channel.durable.v1`. A reader must negotiate it and hold the
current `agent_channel` grant to call `channel.durable`. Its strict typed params
contain an adapter-owned private `credential`, the configured store path hash
`store`, and an `operation` tagged by `action`. See the
[experimental MCP trial](../mcp.md#experimental-sqlite-channel-trial) for actions,
profiles, receipt and retry semantics. This feature does not change v1 channel
reads or their in-memory retention.

Admission checks the live reader, feature, scope, store binding and request ID
before trying a 16-job queue. Typed commands are bounded below 40 KiB each.
SQLite access and serialization run on one worker. No editor-loop wait occurs.
The worker checks the cancellation lease before execution and before replying;
transport uses the same lease. A transaction admitted before revocation may
commit, but its reply is suppressed, leaving a retryable uncertain outcome.
Channel policy is checked again transactionally. Replies belong to the original
connection and request, never to a replacement reader. Private credentials are
redacted in debug output and never returned in tool results.
