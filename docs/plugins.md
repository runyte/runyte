# Experimental plugins

Runyte can run explicitly enabled external programs that register commands and
transform selections. This first capability supports text snapshots and atomic
selected-text replacements. The API is **experimental** and has no stable Rust
ABI. The exact wire version is `runyte-experimental-1`.

## Install and enable the example

Install Python 3 and copy [uppercase.py](plugins/uppercase.py) to a location you
control, for example `~/.local/share/runyte/plugins/uppercase.py`. The script
needs no third-party packages or executable permission when launched by Python.
Add this to your Runyte `config.yaml`, replacing both absolute paths:

```yaml
plugins:
  - id: case
    enabled: true
    executable: /usr/bin/python3
    args:
      - /home/example/.local/share/runyte/plugins/uppercase.py
    bindings:
      uppercase: F12
```

`executable` must be absolute. Arguments are passed directly, without a shell,
`~` expansion, or variable expansion. The working directory is the workspace
root. No plugin directories are scanned. Omitting `plugins`, leaving it empty,
or omitting `enabled: true` starts no plugin process. Configuration is read at
host startup; restarting an existing persistent host is required to load changes.
Enablement applies to every workspace using this configuration.

Select text with the editor's normal selection commands, then run
`:plugin.case.uppercase`, or press `F12` in Normal or Select mode. Multiple
selections are transformed together. For example `éß` becomes `ÉSS`; other text
stays in place. `u` undoes the entire result in one step. A bare caret transforms
the character it operates on, just like built-in selection edits; at EOF its
empty span permits an insertion.

The colon palette lists registered commands and their descriptions. Configured
plugin bindings join the same keymap used by dispatch, view help, and hints.
Bindings use physical key spellings, at most eight keys, and apply in Normal and
Select modes. They are checked against both fast-pane variants and all effective
scopes. A leading plain `1`–`9` or an `Escape`/`Backspace` continuation is
rejected because the modal grammar owns counts and prefix cancellation. A
collision rejects that plugin's registration; it never replaces an existing
binding. `keys.rebind` continues to move built-in defaults only.

`:plugin.case.stop` stops the instance, cancels pending work, and removes its
commands, bindings and subscriptions. There is no live reload or automatic
restart. Restart the workspace host to enable it again. `stop` is a reserved local
command name, supplied by Runyte.

## Asynchronous behavior and errors

The editor captures the invoking buffer, text revision, selections and text before
sending work. You can edit, change panes or close a buffer while a plugin runs.
A result always addresses that captured buffer, including when it is hidden.
Changes to its text, including undo back to identical text, make the result
stale. Closing it invalidates its handle. Moving the selection or changing focus
alone does not invalidate a text result. On success, all current views of that
buffer map their current selections through the same transaction.

The interaction line reports acceptance and updates completion while that
invocation remains its current action; it preserves feedback from newer input.
Rejections and process failures remain available in `:notifications`. A busy
plugin refuses another invocation. Stale results are rejected without edits or
automatic retries; invoke the command again on the current text. A failed start
usually means the executable or script path is wrong. Invalid JSON, incompatible
versions, invalid registration, unknown result IDs, process exit, timeout or
slow delivery stops the instance and removes its registration. A command may
explicitly report a failure and remain available for another invocation.

Standalone mode owns one process per enabled plugin in its workspace host.
Persistent mode starts those processes in the persistent host, once. Detaching,
disconnecting or reattaching a TUI does not restart them or cancel their work.
Pending work can complete while detached. Host shutdown cancels plugin work; a
pending invocation prevents automatic idle retirement until it finishes or times
out. State and plugin processes do not survive a host restart.

## Version and transport contract

This document and the [epoch 1 JSON Schema](plugins/runyte-experimental-1.schema.json)
define the extension contract. `src/protocol/` remains a private
transport for bundled clients, and `headless.rs` remains a testing facade. Neither
is a plugin API. Rust service types are implementation details.

Each API epoch has an exact version string. Incompatible field or semantic changes
require a new epoch and updated documentation; compatibility across experimental
epochs is not promised. Host and plugin must agree before commands become usable.
Plugins should ignore unknown host message fields for compatible additions.
Runyte rejects unknown plugin message variants and fields in this epoch.

### Machine-readable schema

The schema uses [JSON Schema Draft 2020-12](https://json-schema.org/draft/2020-12).
It describes individual decoded messages, not a whole newline-delimited stream
or the plugin configuration file. Its root accepts either direction; use these
definitions when validating one direction or generating API documentation:

| Definition | Direction / purpose |
| --- | --- |
| `#/$defs/hostMessage` | Runyte → plugin stdin |
| `#/$defs/pluginMessage` | Plugin stdout → Runyte |
| `#/$defs/selection` | Unicode coordinates and selection direction |
| `#/$defs/<type>` | A specific message, such as `invoke` or `replace` |

Every message definition includes an example. Required fields, message tags,
status codes, nullability, and unknown-field rules are expressed in the schema.
Host messages permit additional fields; plugin messages and their registration
objects reject them. Tokens remain strings with no numeric format constraint.
The schema's `urn:runyte:plugins:runyte-experimental-1` identifier is a stable
identifier, not a download address; load the checked-in file locally.

Schema validation is not execution validation. JSON Schema's `maxLength` counts
characters, not UTF-8 bytes. `x-runyte-maxUtf8Bytes` and
`x-runyte-maxTotalUtf8Bytes` are documentation annotations; ordinary validators
do not enforce them. The host enforces those byte limits and the encoded 1 MiB
message limit, including its newline. JSON Schema also treats `1.0` as an integer;
emit integer coordinates without a fraction or exponent. JSON strings must contain
Unicode scalar values, with no unpaired surrogate escapes.

The host additionally checks handshake order, command-name uniqueness and binding
collisions, issued handles, pending invocation IDs, revision freshness, selection
bounds and disjointness, primary-index bounds, and replacement count and total
size. In particular, a structurally valid `replace` can receive
`invalid_replacements`. Timing, cancellation, undo, subscriptions and delivery
ordering remain defined by the sections below.

When changing the API, update the schema, this guide, and the wire implementation
together. An incompatible change gets a new epoch string and schema filename;
keep previous epoch schemas available as historical contracts, without implying
that the current host accepts them. Compatible additions update the current
epoch's schema. The schema does not add runtime validation dependencies to Runyte
or require plugins to use a schema validator.

To check the schema, its examples, the guide's JSON blocks, and the runnable
plugin's output, install Python's `jsonschema` package in a development environment
and run from the repository root:

```sh
python3 docs/plugins/check_schema.py
```

The checker also exercises malformed messages and the asymmetric unknown-field
policy. It is a documentation check; the Rust host tests cover stateful protocol
behavior. For an individual message, a Python validator can use
`Draft202012Validator(schema).validate(message)` after loading both JSON values.

### Handshake and framing

Transport is UTF-8 JSON, one object per newline, on stdin/stdout. Flush each
outbound message. Reserve stdout for the protocol. Stderr is discarded in this
initial implementation; debug a plugin separately. Strings containing newlines
must be JSON-escaped. Messages, including the final newline, are at most 1 MiB.

Runyte sends:

```json
{"type":"hello","version":"runyte-experimental-1"}
```

The plugin responds once, within ten seconds:

```json
{"type":"register","version":"runyte-experimental-1","commands":[{"name":"uppercase","description":"Uppercase every selection"}]}
```

Local names and configured plugin IDs contain 1–48 lowercase ASCII letters,
digits or hyphens. A plugin registers 1–16 commands atomically. Names are scoped
as `plugin.<configured-id>.<local-name>`. Duplicate names, built-in collisions,
invalid descriptions, unknown configured binding targets, reserved `stop`, or
binding conflicts reject the entire registration. Descriptions are 1–160 UTF-8
bytes without control characters. Commands cannot be added later in a connection.
Runyte acknowledges successful registration:

```json
{"type":"registered","commands":["plugin.case.uppercase"]}
```

## Captured context and replacements

An invocation includes the entire bounded invoking text and its selections:

```json
{"type":"invoke","invocation":"1","command":"uppercase","buffer":"0","revision":"42","text":"éß 😀","selections":[{"anchor":1,"head":0,"from":0,"to":2},{"anchor":3,"head":3,"from":3,"to":4}],"primary":0}
```

`invocation`, `buffer`, `revision`, and event `sequence` are opaque string tokens.
Do not parse their numeric spelling. Buffer handles are valid only in the current
connection and host lifetime; they identify buffers, not paths or panes. Closing
a buffer ends its lifetime. Reopening the same path does not revive its handle.
Only handles previously supplied in an invocation may be subscribed to. Revisions
are equality tokens: changes advance them, but they need not be consecutive.

All offsets and `primary` are nonnegative JSON integers. Offsets count Unicode
scalar values from the start of the captured text; they do not count UTF-8 bytes,
UTF-16 units, grapheme clusters, lines or display columns. A combining mark is its
own scalar. `from..to` is the authoritative half-open replacement span. `anchor`
and `head` preserve the invoking selection's native direction: `head < anchor`
means backward. They are caret positions, and their min/max need not equal the
operative span: Normal/Select caret semantics include the character at an endpoint,
while some editor selections use half-open semantics. Always slice with `from`
and `to`. `primary` is the zero-based primary selection index in document order.

A successful transformation sends one replacement string per captured selection,
in that same order, within ten seconds of invocation:

```json
{"type":"replace","invocation":"1","replacements":["ÉSS","😀"]}
```

The plugin cannot supply another buffer, revision, range, or selection with the
result. Runyte checks the captured target's lifetime, exact revision, editability,
range bounds, disjoint spans, replacement count and size before mutation. Invalid
results apply nothing. One transaction carries all replacements through the
existing text, syntax, language-server and selection-mapping path. It closes any
previous insert undo group and creates a separate undo step. An empty insertion
at EOF with an empty replacement succeeds without changing the revision or undo.
A replacement identical to a nonempty span still counts as an edit in this epoch.

Runyte returns exactly one completion for a matched result on a live connection:

```json
{"type":"complete","invocation":"1","status":"applied","revision":"43","message":""}
```

Rejection statuses are `stale`, `closed`, `read_only`, `invalid_replacements`, or
`refused`; `revision` is then `null` and `message` explains the refusal. A plugin
can instead send:

```json
{"type":"fail","invocation":"1","message":"Transformation unavailable"}
```

This produces a `complete` with status `failed`, a null revision, and the supplied
message (at most 1,024 bytes, without control characters). Connection failure or
explicit stop cancels unmatched invocations and reports them to the editor;
there is no promise of a final wire message to a dead or stopped process.

## Buffer observation subscriptions

A plugin may subscribe to a buffer handle it received in an invocation:

```json
{"type":"subscribe","request":"watch-1","buffer":"0"}
```

Runyte replies with a current baseline:

```json
{"type":"subscribed","request":"watch-1","buffer":"0","revision":"43"}
```

Each subscription tracks text revision and closure, including ordinary edits,
plugin edits, undo, redo and reload. Selection movement alone is not an event.
Observations are taken at host-turn boundaries, even while detached; changes
within one turn may coalesce. These are invalidation signals, not a transaction
log. Observations contain no text and do not grant a second read/edit operation:
this epoch supplies fresh text only through invocation snapshots.

```json
{"type":"buffer_state","sequence":"1","buffer":"0","revision":"44","closed":false}
```

Sequence tokens increase in delivery order per connection across all watched
buffers. There is no cross-plugin delivery-order guarantee. A plugin transaction's
`complete` precedes its resulting observation. Closure produces a final state
with `closed: true`; the watch can then be explicitly removed. Subscribe again to
an existing live watch to reset its baseline; it does not create duplicate watches.
An unsubscribe is idempotent:

```json
{"type":"unsubscribe","request":"watch-2","buffer":"0"}
{"type":"unsubscribed","request":"watch-2","buffer":"0"}
```

Earlier queued events precede the acknowledgement. No event for that watch is
queued after it. Request IDs are plugin-chosen strings of at most 64 UTF-8 bytes.
The current host also limits the `unsubscribe` buffer string to 64 UTF-8 bytes.
An unknown or closed subscribe target returns
`{"type":"error","request":"watch-1","code":"unknown_buffer"}` or code
`closed`. Slow consumers are disconnected on queue overflow; events are never
silently dropped on a surviving connection. Restart requires a new subscription
and baseline, with no replay of old observations.

## Limits and trust

| Resource | Limit |
| --- | ---: |
| Enabled instances per workspace host | 8 |
| Plugin commands per instance | 16, plus host-supplied `stop` |
| Pending transformations per instance | 1 |
| Captured text | 256 KiB UTF-8; encoded message must also fit |
| Captured selections | 1,024 |
| Total replacement text | 512 KiB UTF-8 |
| Issued buffer handles / watches per connection | 64 |
| Each JSON message with newline | 1 MiB |
| Outbound queued messages per instance | 8 |
| Inbound queued messages across instances | 32 |
| Registration / each invocation deadline | 10 seconds |
| Blocked stdin write deadline | 2 seconds |
| Configured arguments | 32, at most 8 KiB total |

These bounds protect editor queues and the extension data path. They do not cap
an external program's OS memory, CPU, files or descendants. Enabled programs
inherit the host environment and user permissions and can access files and the
network just like programs launched from a shell. Process isolation contains
ordinary plugin crashes; it is **not a security sandbox**. Enable only trusted
programs. Shutdown kills the directly owned child without waiting in the editor
loop; plugins must manage their own subprocesses. Automatic restart, OS resource
sandboxes, package installation, marketplaces, multiple runtimes, arbitrary UI,
general command automation and background reads are outside the delivered API.

Application API development and the opt-in epoch 2 background-job example are
documented in [applications.md](plugins/applications.md). Epoch 1 remains the
default for configurations without an explicit `api` field.
