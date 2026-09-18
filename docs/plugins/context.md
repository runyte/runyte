# Scoped workspace context transport

The external context profile uses the stable `runyte-1` application envelopes
over a separate, opt-in, owner-private Unix socket. It is not the bundled
frontend/control protocol and does not register a plugin process. The required
feature is `runyte.context.v1`. Ordinary process applications and their frozen
fixtures retain their existing contract.

The [context schema](runyte-context-1.schema.json) describes this separate
profile. The current base schema links it without admitting context requests on
ordinary plugin process connections. The independently vendorable
[Python client](context_client.py) uses Python 3.10+ and the standard library;
it does not import the process-plugin SDK.

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
request editing or proposing, with the corresponding read scope. A
`ContextError` exposes its structured `code`. An explicit host refusal preserves
that code and may leave the connection usable. A lost mutation acknowledgement
closes the connection and returns `outcome_unknown`; the client never reconnects
or retries the operation. A caller must not retry an uncertain proposal on a
new connection.

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
{"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.4.0","name":"Local bridge","commands":[],"required_features":["runyte.context.v1"],"optional_features":[],"required_capabilities":["terminal_read"],"optional_capabilities":["editor_context_read"]}
```

The host checks the release range against its actual version and negotiates
features and capabilities using the live native grant. A request cannot grant
itself authority. Commands, settings schemas and ordinary plugin capabilities
are not admitted by this profile. Unknown fields and duplicate fields are
rejected, including inside parameters, changes and registration lists.

## Scopes and methods

| Scope | Methods |
| --- | --- |
| `terminal_read` | `terminal.list`, `terminal.read`, `terminal.snapshot.open/read/close` |
| `editor_context_read` | `buffer.list`, `buffer.read`, `buffer.snapshot.open/read/close`, `selection.get`, `pane.context.list`, `pane.viewport.read` |
| `buffer_edit` | `buffer.edit`, `buffer.append`; requires `editor_context_read` |
| `terminal_propose` | `terminal.input.propose/status/cancel`; requires `terminal_read` |

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

There is no method for direct terminal input, keys, submitting, approving a
proposal, saving a buffer, opening resources, executing commands, changing
selection/focus or changing grants. A `submit` or approval flag is invalid on
every request. The host checks the method allowlist even if a client declares
broader capabilities.

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
| `terminal.input.propose` | `terminal`, `text`, optional `reason` (at most 256 Unicode scalars and 1,024 UTF-8 bytes, no controls) |
| `terminal.input.status`, `terminal.input.cancel` | `proposal` |

Identifiers are opaque, connection-owned ASCII strings with a 256-byte ceiling;
request IDs have a 128-byte ceiling. Handles bind the host incarnation and owner
generation. Reconnecting requires fresh discovery. Names and paths are never
alternate implicit targets. Request IDs are unique during a connection; bounded
deduplication retains at most 1,024 IDs, after which the connection must be
renewed instead of recycling IDs and risking insertion replay.

## Read and edit semantics

Terminal reads capture one emulator state and revision, independently of native
scrolling or review. `screen` excludes scrollback; `tail` returns the newest
retained rows followed by the live screen, in ascending source order. Alternate
screens expose their own current screen without primary history. Partial lines,
blank rows, Unicode and combining marks are preserved; trailing blank cells and
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
64 Ki cells. Snapshot opening uses these same per-turn work bounds; the 4 MiB
terminal snapshot storage ceiling does not authorize a larger synchronous
capture. At most two snapshots are retained per reader; buffer and terminal
snapshots share an 8 MiB reader and 64 MiB host retention ceiling, with 30-second idle
expiry. Host accounting also charges retained snapshots against its shared
terminal payload budget. At most eight readers connect to one host, with
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

## Terminal proposals and revocation

Proposing text does not write terminal input. A native overlay identifies the
reader, exact workspace, terminal and exact proposed text. Each proposal needs
one physical native **Insert text** action; **Reject** writes nothing. Insertion
does not append Enter. Enter used to approve the overlay is consumed by the
overlay. The person submits separately in the terminal.

Only one nonempty line of at most 4 KiB is accepted. CR, LF, C0/C1 controls,
DEL, Unicode line/paragraph separators and escape sequences are rejected as
a whole. Literal backslash-n is ordinary text. The host validates decoded text
again immediately before queue admission; only host-generated bracketed-paste
framing is permitted. Ordinary characters may trigger actions in arbitrary
terminal applications, so insertion is not an execution sandbox.

At most four pending proposals belong to one reader, sixteen to a host; they
expire after two minutes. Presentation captures terminal input generation and
input modes. Approval rechecks these, live grant generation, target incarnation
and physical frontend ownership. Intervening input requires fresh review.
Existing prompts are not interrupted, and detached hosts cannot approve.
Disconnect, terminal exit/close, attachment loss and revocation cancel pending
proposals. A bounded non-interleaved queue supports cancellation before writing
starts. Partial delivery or lost acknowledgements is uncertain and is never
automatically retried.

Session-only grants are the default; remembering a grant is an explicit native
choice. Every workspace has its own grant. Revocation invalidates handles and
snapshots, cancels pending work, drops unsent responses and disconnects readers.
Authorization is checked at request admission, before response publication and
before mutations. Bytes already delivered cannot be recalled. This boundary
controls Runyte's API and is not an operating-system sandbox for same-user
processes.
