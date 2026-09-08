# Application API development

Epoch 2 (`runyte-experimental-2`) is being implemented in the
[application plan](../../context/plans/active/PLAN_PLUGIN_APPLICATIONS.md).
The current implementation supports typed commands, finite background jobs,
retained native views, explicit buffer reads/edits, immutable snapshots and
pane selections. Local filesystem workflows, input forms, remote providers,
subscriptions and managed media backends remain unfinished and are not
advertised capabilities. This is not completion of the application plan.
Epoch 1 remains the default and its uppercase example is unchanged.

Enable the runnable background-job example with absolute paths:

```yaml
plugins:
  - id: jobs
    enabled: true
    api: runyte-experimental-2
    executable: /usr/bin/python3
    args: [/path/to/runyte/docs/plugins/jobs.py]
    capabilities: [jobs]
```

Run `:plugin.jobs.start`, continue editing, and use `:plugin.jobs.cancel` to
cancel the twelve-second task. `:plugin.jobs.stop` stops the owning process and
removes its commands. The host owns one process across persistent-session
attachments. Active jobs protect normal persistent-session quit and idle
retirement; enabled idle processes alone do not. Forced host shutdown discards
live plugin work. The example needs Python 3.10+ and no packages or accounts.
The optional `application.py` client keeps a continuous reader, correlates
responses and dispatches bounded concurrent handlers separately from that reader.

## Native task list

Use the same configuration with `id: tasks`, `args` pointing at `tasks.py`, and
`capabilities: [views]`. Run `:plugin.tasks.open`. `Enter` toggles selected rows;
`Tab` opens actions, including an explicit unfinished-only filter. Ordinary
buffer movement, search, copying, splits, help, Navigator and Finder remain
available. A refresh retains each pane's selection direction and row identity.
Actions against a model that has changed since presentation are refused until
the refreshed frame is prepared.

Views are special buffers backed by bounded semantic models. Purposes are
`document`, `list` and `dashboard`; each row has a stable ID, one line of text
and an `ordinary`, `muted`, `heading`, `warning` or `error` role. Controls and
ANSI sequences are rejected. Warning/error spans use the theme's diagnostic
colors. The bundled frontend protocol is version 51 to carry those new scope
names; the extension epoch remains independent.

Creation stays in the background. `pane.show` requires the ID of a still-pending
invoking command, the same frontend attachment, pane target, and command/input
generation. A detach yields `no_frontend`; changed input/context yields
`context_changed`. An accepted job does not retain that grant. The Python SDK
adds the host request ID to handler context as `invocation`.

`view.publish` replaces an entire model at `expected_revision`. Updates validate
before publication, preserve selections and scroll rows by row ID, and return
`busy` above ten publications per second. No publication timer runs while idle.
A removed row falls back to the nearest remaining row by index. Closing or
ordinary special-buffer eviction produces `view.closed` after any close response.
A stopped plugin leaves its readable view marked `[unavailable]`.

Commands may declare `context: view` and one `primary: true` action. The primary
action defaults to Enter in the application's dynamic key scope. Configured
bindings and primary defaults are checked together with the existing keymap
in both fast-pane variants. The same runtime command metadata supplies the
palette, hints, help and action list. A command can declare up to sixteen
required positional arguments, each with `name` and `type` (`string`, `boolean`,
`integer`, or `enum` with `choices`). Quotes preserve spaces; arguments undergo
no shell evaluation. Boolean spellings are `true`/`false`; integers are signed
64-bit values. Command handlers receive an `arguments` object of typed values.

## Wire contract

UTF-8 newline-delimited JSON, one exact epoch per process. The
[epoch 2 schema](runyte-experimental-2.schema.json) covers implemented messages;
[fixtures](epoch2-fixtures.json) are checked by Python and Rust. Unknown host
fields may be ignored. Plugin envelope and parameter fields are strict. Unknown
methods receive `unsupported`; malformed envelopes terminate the connection.
Required capabilities must be supported **and** explicitly listed in config.
Unavailable optional capabilities are omitted from the granted set. Registration
and configured keymaps are atomic. Capability checks govern host operations and
do not sandbox the trusted process.

Requests in either direction have `type`, `id`, `method`, `params`. A response
contains exactly one `result` or `error`. Host command IDs use `h:`; plugin
request IDs use canonical `p:1`, `p:2`, ... increasing in **send order**. This
concrete encoding implements connection-wide uniqueness with bounded bookkeeping.
Responses may arrive out of order. Reusing or decreasing a request ID terminates
the connection; no request or mutation is automatically replayed after failure.
Resource handles remain opaque strings. They are unique to a connection
generation and must never be parsed, persisted, or used by another owner.

| Capability | Method | Parameters / result |
| --- | --- | --- |
| workspace | `workspace.info` | Empty parameters; workspace display name |
| jobs | `job.create` | Title, deadline in seconds; host-issued job |
| jobs | `job.get` | Job handle; current state and progress |
| jobs | `job.update` | Handle and integer progress 0–100; updated job |
| jobs | `job.finish` | Handle and terminal state; completed job |
| jobs | `job.cancel` | Handle; cancellation requested or existing terminal state |
| views | `view.create/get/publish/close` | Semantic model, owned handle and model revision |
| views | `pane.show` | Pending invocation ID and view handle |
| workspace | `buffer.list` | Offset and page size 1–128; live buffer metadata and next offset |
| workspace | `pane.list` | Empty parameters; pane handles and selection revisions |
| text | `buffer.read` | Buffer, expected revision and scalar range; exact revision-bound text |
| text | `buffer.edit` | Buffer, expected revision and explicit changes; resulting revision |
| text | `buffer.snapshot.open/read/close` | Immutable rope snapshot and explicit scalar chunks |
| selections | `selection.get/set` | Explicit pane, displayed buffer and selection revision |

`command.invoke` is a host request naming the registered command and its declared
`workspace`, `buffer` or `view` context. Workspace commands work on read-only content and
terminals in Normal mode. A response's optional `job` must name an already
reserved job. Successful command acceptance does not mean job completion.
Buffer commands require a live buffer. Each invocation includes its captured
`pane` and `selection_revision`, plus `buffer` and `buffer_revision` when invoked
over a buffer (both are null over a terminal). These handles keep background work
targeted after focus changes. `buffer.list` and `pane.list` issue additional editor
handles; neither operation scans the filesystem.

Jobs start `running`; cancellation moves them to `cancelling`; terminal states
are `succeeded`, `failed`, `cancelled`, and `outcome_unknown`. Terminal transitions
cannot be repeated. Cancellation is idempotent and rejects subsequent success.
Job acceptance and terminal changes generate reliable `job.changed` events;
`job.cancel_requested` requests cooperative cancellation. A surviving connection
gets one terminal transition. Responses precede their resulting events. Connection
sequences increase across these events. Progress is currently recovered through
`job.get`; subscription/coalescing support is still pending.

Control requests time out after ten seconds independently. Finite jobs have
explicit deadlines from one second to one hour. Expiry requests cancellation;
failure to acknowledge cancellation within two seconds stops the owner. Neither
quiescent connections nor jobs with no pending updates create polling timers.
Disconnect destroys live handles and fails outstanding work locally; it cannot
promise a final response to a dead peer or determine a remote mutation's outcome.

The command and the job have separate lifetimes:

```mermaid
sequenceDiagram
    participant Editor as Workspace host
    participant Plugin as Application process
    Editor->>Plugin: command.invoke (h:1)
    Plugin->>Editor: job.create (p:1)
    Editor-->>Plugin: response (p:1), issued job handle
    Editor-->>Plugin: job.changed (running)
    Plugin-->>Editor: response (h:1), accepted job
    Note over Editor,Plugin: Command deadline ends; the finite job continues through detach
    Plugin->>Editor: job.finish (p:2)
    Editor-->>Plugin: response (p:2)
    Editor-->>Plugin: job.changed (terminal)
```

A view's lifetime is independent of that command and job. `view.create` retains
its model, `pane.show` presents it while the invoking foreground grant is valid,
and later `view.publish` requests replace the model without requiring focus.
Closing/evicting the buffer releases its handle and emits `view.closed`.
Stopping the owner releases its live handles while leaving existing view text
readable and unavailable for actions. A new host starts a new generation.

Limits advertised in hello/registered: 64 commands, 16 outstanding host control
requests, four active finite jobs, 1 MiB per encoded line including newline,
32 queued outbound messages and 4 MiB queued payload. Each worker may enqueue
16 incoming messages totalling at most 4 MiB, leaving capacity for other owners.
The most recent bounded set of 64 jobs is available through `job.get`; terminal
history may be evicted, while active jobs are retained. Job titles are bounded
plain text. Protocol bodies and child stderr are not logged by the host.

Run `python3 docs/plugins/check_schema.py` and
`python3 docs/plugins/check_applications.py` with the development-only
`jsonschema` package. Structural checks do not establish ownership, command
collisions, deadlines, queue capacity, revision preconditions or state transitions;
those require the Rust host/worker tests.

## Editor operations and memory

Text offsets are Unicode scalar positions; changes use half-open ranges.
Edits validate liveness, read-only state, revision, ordering, overlaps and size
before one transaction. Each nonempty edit commits a preceding insert group
and adds one undo step. Empty transactions are no-ops. An edit targets its
buffer regardless of focus; current selections in every pane are mapped by
the ordinary editor mutation path. Selection changes are separate: the pane,
displayed buffer and selection revision must all match. A stale selection change
does not roll back an earlier successful text edit.

A read returns exactly its requested range or a structured error. Decoded chunks
are at most 256 KiB and must also fit the encoded line limit; callers shorten
ranges containing many JSON escapes. Edits accept at most 1,024 changes and
512 KiB of replacement text. Up to two immutable snapshots of at most 16 MiB
are retained per owner. Reading renews a 30-second idle deadline; closing the
source buffer, explicit release, expiry or disconnect releases the snapshot.
An expired snapshot returns `closed`. Chunks never combine different revisions.

An instance can hold sixteen native views, each with at most 10,000 rows.
Single-message models must fit the encoded line with envelope headroom;
staged 4 MiB models and row patches are not implemented yet. View projections
and immutable snapshots share a 48 MiB retained payload allowance per plugin
and 160 MiB across the host. The remaining portions of the plan's 64/256 MiB
budgets are reserved for bounded queues, decoding and publication copies.
These measure payload, not allocator RSS or the external process's memory.
Buffer/pane issuance is bounded at 1,024/128 handles per connection generation.

Still required by the active plan: local document/filesystem operations and
confirmed mutations; prompts/forms with secret handling; source subscriptions
and resynchronization; provider reads, asynchronous saves and transfer outcomes;
row patches and staged publication; managed helpers, activity leases, state,
settings and a plugin manager; SFTP/FTP and media examples; broader SDK/conformance
coverage and the complete performance/platform acceptance matrix.
