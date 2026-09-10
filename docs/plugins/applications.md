# Application API development

Epoch 2 (`runyte-experimental-2`) is implemented and validated as recorded in the
[completed application plan](../../context/plans/completed/PLAN_PLUGIN_APPLICATIONS.md).
The current implementation supports typed commands, finite background jobs,
retained native views, explicit buffer reads/edits, immutable snapshots and
pane selections, local metadata/browsing, document lifecycle operations, native
input with asynchronous field validation, confirmed bounded recursive filesystem
mutations, private binary download staging, confirmed disk-file uploads and provider-backed
UTF-8 document opening, conditional remote saves, native confirmation of weaker
overwrites, explicit rebind and remote conflict inspection. Runnable SFTP and
FTP/FTPS adapters share a native remote browser with explicit transport and
overwrite guarantees. Metadata subscriptions provide consistent baselines,
ordered changes and explicit resynchronization. Managed helpers, continuing
activity leases, settings/state, a native manager and the local mpv controller
support continuing applications. The API remains experimental.
Epoch 1 remains the default and its uppercase example is unchanged.

The [authoring guide](authoring.md) provides clean-checkout setup, method/event
and capability indexes, lifetime rules and epoch migration. The
[conformance guide](conformance.md) maps tests to their behavior boundaries.
The dependency-free [Node checklist](tasks.mjs) demonstrates the same public
protocol without the Python client.

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
Cancellation callbacks have their own bounded worker so waiting command handlers
cannot starve them. Keep cancellation callbacks short: signal the running work
and return, without waiting for a command handler or its lock.
The client also has a bounded, nonblocking output lane: sixteen queued/in-flight
frames and 4 MiB of encoded bytes. A stalled host reader does not hold the reply
correlation lock or stop cancellation/EOF processing. Request deadlines include
output admission. An SDK delivery timeout retires the connection so an unsent
frame cannot become a late mutation; explicitly restart the plugin before making
new requests. An ordinary error response from the host, including `timeout`,
does not itself retire the connection. State mutations with an uncertain reply
retain `outcome_unknown` and are never replayed automatically.

## Plugin manager

`:plugins` opens the native manager for configured plugins, including disabled
and failed entries. Its details show lifecycle state, negotiated capabilities,
active jobs and activity leases, helpers, pending cleanup and a bounded diagnostic
summary. Enter opens the selected entry's available lifecycle actions. The manager
does not read disabled bundles or expose configured settings, arguments or raw
process output.
Disabled entries must first be enabled in configuration before they can restart.
Configuration is bounded at 128 entries with at most eight enabled workers;
exceeding either limit reports a configuration failure without starting workers.

`:plugin-stop <configured-id>` stops an owner and
`:plugin-restart <configured-id>` explicitly requests a fresh connection.
Restart waits for the old process and its pending work to finish cleanup before
launching the replacement. Stop cancels a pending restart. Failures never trigger
automatic restart, and a new connection never replays old commands or jobs.
Pending restart and cleanup protect ordinary quit and idle retirement. If process
cleanup cannot be verified, the failed entry retains its slot and refuses restart.

Stopping removes commands, bindings, subscriptions and input surfaces. Retained
application views stay readable and show `[unavailable]`; dirty provider documents
remain editable, with remote saving unavailable until an explicit provider rebind.
Editor-owned terminal sessions survive plugin stop. Saved workspace preferences
remain on disk; live handles belong to the old connection and cannot be reused.
Persistent-session detach retains the same manager and workers without restarting
them. An unchanged manager starts no refresh timer or redraw loop.

## Native task list

The [Python, Rust and C todo showcase](todo/README.md) expands this example into
three independent applications with matching add, toggle, remove and filter
commands, reproducible build instructions and shared behavior checks.

Use the same configuration with `id: tasks`, `args` pointing at `tasks.py`, and
`capabilities: [views]`. Run `:plugin.tasks.open`. `Enter` toggles selected rows;
`Tab` opens actions, including an explicit unfinished-only filter. Ordinary
buffer movement, search, copying, splits, help, Navigator and Finder remain
available. A refresh retains each pane's selection direction and row identity.
Actions against a model that has changed since presentation are refused until
the refreshed frame is prepared.
Choosing an action that declares positional arguments opens the command palette
with its full name and a trailing space. Enter the arguments there, then press
Enter to invoke it; Escape cancels. Actions without arguments invoke directly.

Views are special buffers backed by bounded semantic models. Purposes are
`document`, `list` and `dashboard`; each row has a stable ID, one line of text
and an `ordinary`, `muted`, `heading`, `warning` or `error` role. Controls and
ANSI sequences are rejected. Warning/error spans use the theme's diagnostic
colors. The bundled frontend protocol is version 52, including application
activity in persistent-session health; the extension epoch remains independent.

Creation stays in the background. `pane.show` requires the ID of a still-pending
invoking command, the same frontend attachment, pane target, and command/input
generation. A detach yields `no_frontend`; changed input/context yields
`context_changed`. An accepted job does not retain that grant. The Python SDK
adds the host request ID to handler context as `invocation`.

`view.publish` replaces an entire model at `expected_revision`. Updates validate
before publication, preserve selections and scroll rows by row ID, and return
`busy` above ten publications per second. No publication timer runs while idle.
A removed row falls back to the nearest surviving row in the previous order,
with the lower previous index breaking ties. Selection direction is preserved
even when a reorder crosses its endpoints. Closing or
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

Both `hello` and `registered` advertise effective control, job, helper, activity,
settings and state limits. Their optional `limits.resources` inventory adds view,
model, text/snapshot, subscription, input and retained-payload ceilings. Current
hosts always provide it; older epoch 2 hosts may omit it. Use the documented epoch
defaults when it is absent, and ignore unknown host fields for compatible
additions. Counts are per owner except `plugin_instances` and
`host_retained_payload_bytes` (per host), and `input_surfaces` (per attached
frontend). Byte fields count UTF-8 or encoded payload as specified by the relevant
operation; the inventory does not override wire-envelope or temporary-reservation
limits and does not promise that all independently valid resources fit together.
`watched_sources` bounds aggregate subscription/source pairs as well as the unique
source table; `observation_state_bytes` bounds one captured source's metadata.

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
| processes | `process.start/get/read/write/close` | Managed argument-vector helper, bounded binary pipes and explicit cleanup |
| terminals | `terminal.open` | Foreground handoff to a native terminal session with exact arguments |
| external | `external.open` | One foreground system-handler handoff for a URL or workspace file |
| notifications | `notification.publish` | Bounded owner-labelled feedback without taking focus |
| activity | `activity.acquire/renew/get/release/cancel` | Renewable continuing-work lease, explicit cleanup and expiry |
| settings | `settings.get` | Immutable validated configuration for this owner |
| state | `state.get/set/delete` | Versioned nonsecret workspace document with conditional atomic replacement |
| views | `view.create/get/publish/patch/close` | Semantic model, owned handle and model revision |
| views | `view.query.set` | Explicit revision-bound query intent; matching publication settles it |
| views | `view.stage.open/write/commit/close` | Bounded construction and atomic publication |
| views | `view.snapshot.open/read/close` | Immutable canonical model JSON and UTF-8 byte chunks |
| views | `pane.show` | Pending invocation ID and view handle |
| workspace | `buffer.list` | Offset and page size 1–128; live buffer metadata and next offset |
| workspace | `pane.list` | Empty parameters; pane handles and selection revisions |
| source-dependent | `event.subscribe/resync/unsubscribe` | Explicit source filters; subscription handle, ordered baseline and changes |
| text | `buffer.read` | Buffer, expected revision and scalar range; exact revision-bound text |
| text | `buffer.edit` | Buffer, expected revision and explicit changes; resulting revision |
| text | `buffer.snapshot.open/read/close` | Immutable rope snapshot and explicit scalar chunks |
| selections | `selection.get/set` | Explicit pane, displayed buffer and selection revision |
| filesystem | `filesystem.stat` | Workspace-relative path and optional metadata revision; shallow kind, byte length and opaque revision |
| filesystem | `filesystem.list` | Workspace-relative path, offset, limit and optional expected revision; retained directory handle and metadata page |
| filesystem | `filesystem.prepare` | Directory handle, expected revision and typed intent; owned prepared plan and descriptions |
| filesystem + jobs | `filesystem.apply` | Plan and invoking command; presents native confirmation, with no immediate filesystem mutation |
| filesystem | `filesystem.cancel/release` | Cancel a plan or release a directory snapshot, idempotently |
| filesystem + jobs | `staging.create` | Running owned job and exact byte count; private staging handle and writable path |
| filesystem + jobs | `staging.prepare` | Staging handle, directory/revision, destination and SHA-256; seals bytes and returns a filesystem plan |
| filesystem | `staging.close` | Releases an owned unprepared staging handle |
| providers | `provider.register` | Unique provider name and independent conditional-write/atomic-replace declarations |
| documents + jobs | `resource.open` | Configured plugin, provider, resource key and optional invocation; host-owned open job |
| documents + jobs | `resource.rebind` | Buffer and expected revision; bounded remote reconciliation job |
| documents + jobs | `resource.inspect` | Buffer, expected revision and foreground invocation; fresh remote comparison job |
| documents | `buffer.open` | Existing workspace-relative text path and optional invoking command; explicit buffer/revision |
| documents | `buffer.create` | New workspace-relative path, initial text and optional invocation; named unsaved document |
| documents + jobs | `buffer.save` | Owned buffer and expected revision; asynchronous host-owned save job |
| documents | `buffer.close` | Owned buffer and expected revision; closes only a clean document without a pending write |

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
sequences increase across these events. Inspect progress through `job.get` or
subscribe to the job metadata source for ordered, coalesced observations and
explicit resynchronization after dropped observations.

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
    Note over Editor,Plugin: Command deadline ends while the finite job continues through detach
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
Models contain at most 4 MiB of canonical JSON and project to at most 4 MiB of
text. Single-message models must fit the encoded line with envelope headroom;
larger models use the staging operations described below. View projections
and immutable snapshots share a 48 MiB retained payload allowance per plugin
and 160 MiB across the host. The remaining portions of the plan's 64/256 MiB
budgets are reserved for bounded queues, decoding and publication copies.
These measure payload, not allocator RSS or the external process's memory.
Buffer/pane issuance is bounded at 1,024/128 handles per connection generation.

The media examples, SDK/conformance coverage and planned performance/platform
acceptance are delivered. See the [validation record](../../context/plans/completed/PLAN_PLUGIN_APPLICATIONS.md#native-platform-acceptance--2026-09-09)
for the accepted commit, tested targets and limits of that evidence.

## Columns, blocks and atomic model updates

`dashboard.py` is a runnable example with capability `views`. Its `open` command
shows twelve rows; `large` stages 8,000 rows. Enter toggles a row, and Tab offers
reverse, inspect and immutable-model verification actions. There is no idle
polling. Configure it like `tasks.py`, using ID `dashboard` and its script path.

Models optionally contain up to eight `columns: [{id,label}]`. Every row then
has empty `text` and exactly one `{text,role}` cell per column. Without columns,
rows use their original single-line text and have no cells. Runyte clips each
column at 32 terminal cells on grapheme boundaries, retaining the complete value
in the model. Optional `detail` and `preview` blocks are `{text,role}`, at most
64 KiB and 1,024 lines each; `status` uses the same shape but one line and 1,024
bytes. Line feeds are permitted in multiline blocks; other control characters
are rejected. Headers and blocks do not identify selectable data rows. Enter
requires a selected data row; other view commands remain available on empty
views. An optional `actions` list restricts view commands to those registered
local names; an omitted or empty list retains all registered view commands.

`view.patch` takes `view`, `expected_revision`, optional complete `header`
(the model without `rows`) and at most 1,024 `operations`. Operations are
`{kind:"insert",before:row_id_or_null,row}`, `{kind:"update",row}`,
`{kind:"remove",ids}` and `{kind:"reorder",ids}`. Insert with null appends;
update identifies the existing row by its unchanged ID. Reorder must name every
current row exactly once. Operations run in order on a candidate model and
publish atomically only if the whole candidate validates. Errors preserve the
old model and revision. Concurrent preparations for the same view return `busy`.
A patch may reference at most 20,000 row IDs across all operations (including
insert anchors), and each ID list holds at most 10,000 entries. Structural array
limits are enforced while decoding: oversized inline arrays are protocol errors,
and oversized staged arrays refuse the candidate before excess values are retained.

For larger updates, `view.stage.open` captures a view and `expected_revision`,
`kind: model|patch`, and exact UTF-8 `bytes` (1 byte–4 MiB). Write JSON text with
`view.stage.write {stage,offset,text}` in contiguous chunks of at most 128 KiB;
each reply gives the next offset. A staged patch is `{header?,operations}`.
`view.stage.commit` prepares and publishes the complete document once; stale
revisions, malformed candidates and cancelled stages never partly publish.
Incomplete commits retain the stage for further writes. Two stages may exist
per owner and writes renew a 30-second idle expiry. Closing a stage also
cancels its already admitted, unfinished publication. Creation can start with
a small empty view followed by staged publication.

`view.get` and publication return `{view,revision,model}` for small models;
large models return `{view,revision,bytes,rows}`. To read a large model, open
`view.snapshot.open {view,expected_revision}` and use its `{snapshot,revision,bytes}`
result. Read with `{snapshot,offset,limit}` (limit 1–128 KiB); replies are
`{offset,text,eof}`, with UTF-8 byte offsets. Chunks come from one immutable
canonical JSON encoding even if the live view changes. Two snapshots per owner
have 30-second refreshed idle expiry. Closing the view or owner releases them;
`view.snapshot.close` is idempotent. Unknown reads return `not_found`.

The Python SDK's `publish_model`, `patch_view` and `get_model` choose the bounded
inline or chunked path and close temporary handles. They do not retry a refused
publication. Model validation, JSON encoding, projection, row maps and text
transaction preparation run outside the editor loop. Installation checks the
captured model and buffer revisions and maps the selections and viewport that
exist at commit time. The host reserves preparation memory before work starts,
shares its finite worker admission bound with local filesystem work, and retains
stopped-owner reservations until actual worker completion.

## Explicit queries and observed views

`catalog.py` demonstrates explicit filtering over 5,000 deterministic local
records with capabilities `views` and `interaction`. Run `:plugin.catalog.open`,
then Tab → Filter. The native prompt sends no query while typing; Enter starts a
finite lookup and empty text restores all records. Refresh explicitly retries.
One worker and one replaceable latest intent bound concurrent lookup work. There
is no background timer while settled. The Observations action displays the last
viewport endpoints and accepted-action metadata. Configure it like `dashboard.py`
with ID `catalog` and the catalog script path.

`view.query.set {view,expected_revision,text,expected_query_revision?}` starts or
replaces query intent. `text` is at most 1,024 UTF-8 bytes, allows empty text and
rejects controls. The result is `{view,revision,query:{revision,text,pending}}`;
the outer revision still identifies the model. The first query enables this
metadata. Later sets require the exact previous `expected_query_revision`, and
every accepted set advances it, including a retry with identical text. The host
adds a pending label to the view title while retaining the old rows and text
revision. Query state is shared by every pane showing that view.

Once a query exists, every `view.publish`, `view.patch` and `view.stage.open`
requires its exact `expected_query_revision`, alongside the model revision.
Staging captures the query token when opened; it cannot acquire a newer token
at commit. Background preparation captures the token too, including the absence
of a query. A query change before completion makes that result stale even when
the text returns to the same value. Only a successful matching publication clears
`pending`. Cancellation, invalid models, pacing and failures preserve the current
intent and old rows. Each activated query reserves 4 KiB inside the owner's
retained-payload budget. Legacy views keep their existing tokenless calls.

Primary row actions are refused while a query is pending. Other view commands
remain available with `rows: []`, allowing Filter and Refresh to replace or retry
the query. Handlers for other row operations must continue to require selected
IDs. Invocations include optional `query_revision`; action menus capture it and
refuse entries from an older query. Matching publication still needs a prepared
frame before actions can use the new model. The SDK's `set_query`, `publish_model`
and `patch_view` expose these preconditions without automatic retries.

Subscribe to `{"kind":"viewport","view":"…","pane":"…"}` using an owned view
and an issued pane handle. Its state contains `model_revision`, `visible`, `top`
and `bottom`; endpoints are stable data-row IDs or null. The host derives them
from final wrapped/folded/diff-aware prepared rows, excluding headers and padding.
Two panes can therefore report different endpoints for one model. An unavailable
prepared model has a null revision; the host never interprets old frame rows
through a newer projection. Hidden, covered, maximized-away and detached panes
report no visible endpoints. Closing the pane or view produces a closed source.
Only watched viewports are scanned; changes use the existing coalescible queue
and do not create timers or request another frame.

`{"kind":"view_actions","view":"…"}` observes accepted commands. Its baseline
contains an `accepted` counter, not an action history. Reliable `event.action`
contains `{subscription,source,revision,action}`. The action has `id`, `request`,
`command`, `pane`, `model_revision`, optional `query_revision`, `selection_revision`
and `selected_count`. The `request` identifies the existing `command.invoke`,
which alone carries the full selected IDs and arguments. An observation does not
invoke the command again or grant foreground authority. Rejected dispatches and
later command results do not create actions. Queue capacity is checked before
acceptance; an oversized callback is refused without stopping its cooperative
owner. Reliable delivery failure stops the owner. Accepted actions retain FIFO
order through state coalescing, resynchronization and unsubscribe acknowledgement.
Both new source kinds require `views` and remain owner-scoped.

## Local file manager

Enable the checked-in `files.py` beside `application.py`:

```yaml
plugins:
  - id: files
    enabled: true
    api: runyte-experimental-2
    executable: /usr/bin/python3
    args: [/path/to/runyte/docs/plugins/files.py]
    capabilities: [views, filesystem, documents, interaction, jobs]
```

Run `:plugin.files.open .`. Enter opens the selected regular text file or browses
the selected directory; Tab offers Parent, Refresh, Trash and native destination
prompts for New, New directory, Rename selected and Copy selected. Destination-taking
actions use the colon palette: `:plugin.files.create "new note.txt"`,
`:plugin.files.mkdir subdir`, `:plugin.files.rename renamed.txt`, and
`:plugin.files.copy copy.txt`. Destinations are relative to the directory shown.
Rename, Copy and Trash operate on exactly one selected entry. The native
confirmation shows the proposed operations; Enter applies with trash semantics,
Escape cancels, and its existing `P` key explicitly chooses permanent deletion.
There is no background polling. Refresh reads again, and a completed filesystem
operation refreshes the retained view without taking focus.

The local API accepts UTF-8 workspace-relative paths without `..`, absolute
paths or controls. Existing symlink components must resolve inside the workspace.
This reuses project-path checks and the existing `FsPlan` boundary; it does not
resolve the deferred hostile-process symlink-race issue or sandbox the plugin.
The example browses ordinary directories and opens ordinary UTF-8 text files;
it labels symlinks without following them as actions.

`filesystem.stat` performs shallow metadata IO, so a large directory does not
require listing its children. It resolves symlink aliases only inside the workspace,
returns the requested relative path, kind, byte length and an opaque metadata
revision, and rejects a mismatching optional `expected_revision` with `stale`.
It retains no directory handle; byte length for a directory is filesystem metadata,
not the summed contents. Revisions describe observations and are not mutation
permissions. The manager uses stat before entering/opening a row; mutations still
require a prepared directory plan and native confirmation.

Directory reads, plan preparation and document loading run on a bounded blocking
pool, with two pending local requests per owner and sixteen across the host.
Reads and results retain their slot even if the owner stops before IO finishes;
late results cannot recreate resources for that owner. Each directory snapshot
contains at most 1,024 entries, including dotfiles. Pages contain at most 128
entries; pass `expected_revision` from the first page to reject changes between
pages. At most two directory snapshots and two prepared plans are retained per
owner. `filesystem.release` releases a snapshot; `filesystem.cancel` releases an
unpresented plan or dismisses that owner's confirmation. Each snapshot, plan or
preparation reserves 8 MiB within the shared retained-payload allowance.

Intents are `create_file`, `create_directory`, `rename`, `copy`, and `trash`.
The latter three name an entry from the issued directory and admit regular files
of at most 8 MiB or directories with bounded descendants. Directory sources permit
at most 1,024 entries including the root, 32 descendant path components, 64 MiB of
regular-file data and 4 MiB of captured path/fingerprint metadata. Symlink targets
are counted as metadata and copied as links without following them; the top-level
source remains an ordinary file or directory. The byte budget covers regular-file
data, separately from native macOS extended attributes/resource forks and ACLs.
Metadata copying retains the platform's existing behavior.

Preparation and application revalidation enforce the traversal limits. Copying
also charges entries/depth/metadata as it walks and refuses data beyond each
file's validated size, so growth cannot expand a staged copy without limit.
Failure retains the existing collision/cleanup/recovery behavior. Directory
rename retargets open descendants while preserving newer unsaved text and undo.
Preparation uses the existing directory baseline and plan collision checks. `filesystem.apply` only opens the host's confirmation, requires
a current foreground grant, and returns `busy` while another input surface owns
the frontend. The plugin has no API that supplies the user's confirmation.
Accepting the confirmation starts a host-owned job and emits `job.changed` followed
by `filesystem.started {plan, job}`. Filesystem application requires `jobs` as well
as `filesystem`. Capacity is checked at acceptance; when no worker or payload
budget is available, the confirmation stays visible for retry. Human waiting has
no job deadline. The worker uses the existing `FsPlan` collision, trash and recovery
logic, then prepares moved-file baselines and bounded directory refreshes off the
editor loop. Completion installs those facts only against matching identities and
revisions; newer text edits and dirty explorer proposals remain intact.

One filesystem apply can run at a time. It retains the plan's 8 MiB charge and an
additional 16 MiB through completion, including owner stop. Reconciliation admits
at most 128 live filesystem buffers, 1,024 rows per refreshed directory and 4 MiB
of retained directory refresh payload. Oversized refreshes retain the explorer and
report that it needs manual refresh. New document opens/publication and all saves
are refused during apply; existing `--wait` paths may be reused without reload.
Filesystem-buffer close/reload/discard and ordinary/force quit wait for completion.
Scratch and generated-view retirement, text editing and detach remain available.

File operations are not editor undo steps. `filesystem.finished` reports
`succeeded`, `failed`, `cancelled` or `outcome_unknown`, an applied-operation count
and whether recovery artifacts remain. Host notifications retain recovery paths
and reasons. Dismissal, detach, source-buffer closure and owner stop cancel a
*displayed* plan. Once accepted, the host retains responsibility for reconciliation
even while detached or after the owner stops.

`job.cancel` accepts cancellation only while this host operation is queued. Its
atomic transition races safely with worker startup; once disk mutation begins it
returns `conflict` and the eventual report states what happened. The 60-second
control deadline can cancel queued work; it cannot interrupt a running OS mutation.
Running work remains protected until its result arrives and never requires a
plugin acknowledgement. Worker failure without a report is an unknown outcome:
inspect the workspace before retrying, and reconcile preserved dirty buffers.
No failed or uncertain mutation is automatically replayed.

`buffer.open` reads at most 8 MiB using the ordinary text/binary classifier and
disk baseline. It reuses an existing buffer, preserving unsaved text. Omitting
`invocation` opens in the background; supplying it requests presentation in the
originating pane. A changed foreground context rejects publication without
switching panes. Opening an existing file does not save, reload or close it.


## Native input

The `interaction` capability admits `ui.prompt`, `ui.pick`, `ui.form`,
`ui.confirm` and `ui.dismiss`. Every opening request names a current foreground
`invocation`. A second surface returns `busy`; macro recording/replay also refuses
input acquisition. Each retained surface reserves 512 KiB in the shared payload
ledger until completion or cancellation. Forms have at most sixteen uniquely named fields: `text`,
`secret`, `boolean` or `choice`. Text fields support `required`, `minimum_length`
and `maximum_length` (Unicode scalars), with an additional 4,096-byte value limit.
Choices contain at most 64 distinct bounded labels. Titles/labels are plain text
without controls. Values start empty, false, or at the first choice.

Opening returns `{ "surface": "opaque handle" }` promptly. The calling command
can finish; human input does not extend a control deadline. Completion delivers a
new host request, `ui.submit`, with `{surface, accepted, values}`. The Python SDK
routes this to `app.on_input(context)`, adding `invocation` for the new request.
Acknowledge it with the ordinary command response (the SDK does this). Accepted
input carries a fresh foreground grant valid for that callback. Cancellation
returns empty values and grants no authority to reopen UI. Dismissal is owner-only
and idempotent; detach, source closure, competing native input and owner failure
cancel the surface. A dead owner receives no callback. The callback has the same
ten-second deadline as other control handlers.

Forms use Tab/Shift-Tab or Up/Down for fields, Left/Right/Space for choices and
booleans, Enter to submit valid values, and Escape/Ctrl-c to cancel. Text editing
uses scalar cursor movement, Home/End, Backspace/Delete and bounded literal paste.
`ui.pick` displays and filters candidates using the ordinary picker matcher;
Up/Down selects and Enter accepts. `ui.confirm` uses the native confirmation vocabulary: Enter accepts with
`confirmed: true`, while Escape/Ctrl-c cancels with no values.
The local file manager uses `ui.prompt` followed by native filesystem confirmation.

Secret values appear in the accepted owner callback and, for fields explicitly
opting into asynchronous validation, in an owner-only check after physical Enter.
All editor snapshots
and bundled frontend frames contain masking glyphs; macro recording, command
history, editor text and diagnostic input tracing never retain the typed value.
Input state is ephemeral and is not persisted. Plugins must keep secret values
out of their own logs and state. Validation errors and secret-bearing submit
callback errors use generic host feedback instead of plugin-provided error text.

### Asynchronous field validation

A field may declare `validate: true` and an optional static `validation_message`
(up to 160 plain UTF-8 bytes). After physical, unmodified Enter passes all local
field checks, the host sends `ui.validate {surface, revision, fields, values}`.
`fields` contains exactly the opted-in IDs. `values` contains every nonsecret
field for cross-field checks, plus only those secret fields with `validate: true`.
Typing, moving between fields, copying snapshots and cancellation send no values.
A validation request provides no foreground authority.

Reply with `{kind:"validation", surface, revision, fields:[{field,status}]}`,
using each requested field exactly once and statuses `valid`, `invalid` or
`unavailable`. The surface and revision must match the request. A successful
result submits automatically only while the original Enter intent, complete form
revision and foreground context remain unchanged. Any subsequent input cancels
that submit intent. Every value edit advances the revision, even editing away
and back to identical text. Stale replies cannot submit or replace current-value
feedback. Current-revision feedback never steals field focus after navigation.

One validation runs at a time per application, with one latest explicit Enter
intent waiting behind it. Validation shares the sixteen control-request slots;
a queued intent is admitted when capacity returns. The editor remains editable.
Cancellation or value changes emit reliable `ui.validation_cancelled` with the
request, surface and revision; cooperative validators should abandon work and
still settle their callback. No value appears in that event. A ten-second timeout
marks validation unavailable and leaves the form open for retry. Up to sixteen
known late request IDs are retained without values; reaching that limit refuses
further validation until a reply retires an ID or the owner restarts. Waiting for
human input has no deadline and no polling timer.

Invalid fields use their predeclared message or generic text. Unavailable checks
and callback failures use fixed retry feedback; remote exception strings never
enter the form or diagnostic snapshots. A malformed result is a protocol error.
Cancellation, detach, input takeover, source closure and owner stop prevent late
results from reopening or submitting a surface.

The Python SDK dispatches `app.on_validate(context)` on its own bounded worker.
Return a mapping from requested field ID to status; the SDK constructs the typed
response and preserves correlation. It keeps validation cancellation on the
reserved control worker, so a validator waiting on IO cannot block cancellation.
Use bounded IO and keep secrets out of plugin logs.

For an account-free demonstration, configure `validation.py` with the
`interaction` capability and run `:plugin.validation.open`. Enter any name except
`taken` and the example code `demo-code`, then press Enter. The example simulates
a 200 ms service check; editing during that check keeps the form open. It has no
accounts, network dependencies or background polling.

### Explicit document lifecycle

`buffer.create` creates a named unsaved local document with up to 512 KiB of initial
UTF-8 text. The path must be inside the workspace, absent from disk, and not owned
by another live buffer. It returns the same buffer/revision result as `buffer.open`.
Neither operation changes focus without a valid optional invocation grant. Creating
the document does not write a file; the normal editor save command or `buffer.save`
persists it later, refusing a file that appeared in the meantime.

`buffer.save` requires both `documents` and `jobs`. It validates the explicit
buffer and expected revision before applying the ordinary trailing-whitespace
save hook as an undoable transaction, then captures immutable text and disk
identity. At most 8 MiB of document text is supported. Disk IO runs off the editor
loop, shares the 16 local worker slots, and reserves 16 MiB of the retained payload
budget until completion. The immediate result is a host-owned job with a 60-second
deadline. Only host IO completion may finish/update that job. The final
`job.changed` event is also recoverable with `job.get`.

A successful save advances the saved baseline to the captured text; edits made
while it was running stay dirty, and undoing those edits returns to the saved
baseline. External changes are checked before replacement. Save warnings retain
the ordinary notification details, including any recovery location. A missing
verification result keeps the buffer dirty. Matching background file observations
cannot clear an uncertain outcome; explicitly inspect and reload, discard, or save
to reconcile it.

A second save, close, reload, discard, filesystem-plan application, normal quit or
force-quit command is refused while a document write is pending. Detaching leaves
the host and write running. Explicit host termination can still interrupt it.
Cancellation and deadline expiry request cancellation without requiring a plugin
acknowledgement. They cannot roll back an OS write already in flight: protection
and payload accounting remain until IO settles. If that write may have committed,
the job becomes `outcome_unknown` and the text remains dirty. Stopping the plugin
likewise preserves the pending write's protection until the host receives its
result. `--wait` does not complete a pending or dirty document.

`buffer.close` has no force option. It checks revision, pending writes and dirty
state before using ordinary buffer retirement, so plugins cannot silently discard
user changes.

`documents.py` is a small public-operation example with workspace `create` and
buffer-context `save`/`close` commands. Configure it like `files.py`, using
`id: documents`, `args: [docs/plugins/documents.py]` and
`capabilities: [documents, jobs]`. For example,
`:plugin.documents.create notes.txt "first note"` opens the new unsaved document;
`:plugin.documents.save` returns its save job. Normal `:write`, movement, editing
and buffer management remain available.


## Provider-backed documents

`memory.py` is a deterministic multi-chunk provider with no network or storage.
Configure its plugin ID as `memory`, executable as `python3`, argument as the
absolute path to `docs/plugins/memory.py`, epoch as `runyte-experimental-2` and
capabilities as `[providers, documents, jobs]`. Run `:plugin.memory.open notes`;
`alias` resolves to the same live document. The document supports normal editing,
search, selection, splits, undo and syntax highlighting. Newline bytes are
preserved, including CRLF. Native `:write`, `:wq` and `:write-buffer-close`, plus
`:plugin.memory.save`, use the conditional upload protocol below. Normal save
trimming hooks still apply. `:plugin.memory.rebind` explicitly reconciles the
current provider document. `:plugin.memory.inspect` and native `:diff-remote`
compare fresh remote text with the editable document. Native `:reload` obtains
a fresh remote version and offers recovery choices for dirty or uncertain text,
as described below. Local write-to-path remains refused. Native writes to weaker providers require
foreground confirmation. Discard restores the accepted in-memory baseline, preserving any
unknown write and its dirty protection. Stopping a provider leaves editable text
marked unavailable. The memory example resets remote content on process restart.

Add `--weak` after the memory script path in its configured argument list to
advertise `conditional_write: false` while retaining `atomic_replace: true`.
Native saves then show the overwrite confirmation described below. The memory
implementation still compares and replaces under its local lock; this mode
demonstrates a weaker capability declaration without using a remote account.
`:plugin.memory.save` is refused in this mode because plugin-facing `buffer.save`
does not yet carry foreground overwrite approval.

An instance grants `providers` before `provider.register {name, conditional_write,
atomic_replace}` can register up to eight unique names. Declarations describe
transport capabilities independently. Plugin-facing saves require conditional
writes; native saves can request explicit confirmation for weaker providers.
Atomic replacement is a separate guarantee. `resource.open {plugin, provider, key, invocation?}` requires
`documents` and `jobs` on the requesting application. The provider can be another
configured application. Its generation, provider name and canonical key are
correlated independently from the requesting application's job and buffer handle.
A resource key is opaque: it never becomes a local path, Git target or LSP URI.
Labels must be safe display text without credentials; resource keys are not
included in titles. An optional syntax hint names a Runyte language.

Opening accepts a finite 60-second host-owned job. The host sends the provider
`resource.stat {job, provider, key}` and then serial `resource.read {job, provider,
key, version, offset, limit}` calls, each with a ten-second control deadline.
Provider calls may arrive before the open request's response; the authoring client
keeps reading continuously and dispatches resource handlers separately from
waiting command handlers. Responses use the ordinary `response` envelope, with
`result: {kind: "stat", value: metadata}` or `{kind: "read", value: chunk}`;
ordinary typed errors are accepted in either phase. Metadata contains `key`,
`label`, optional `syntax_hint`, `version`, `encoding: "utf-8"` and `bytes`.
Each chunk contains `version`, byte `offset`, UTF-8 `text` and `eof`.

A document is at most 8 MiB. Each decoded chunk is at most 128 KiB, with the
independent 1 MiB encoded-frame limit. Offsets and sizes count UTF-8 bytes, not
Unicode scalar positions; editor text methods continue to count scalars. Chunks
must preserve the advertised version, begin at the requested byte offset, make
positive progress before EOF, and finish at exactly the advertised size. NUL and
unsupported encodings reject publication. Resource keys/versions/labels are bounded
at 4,096/256/160 bytes. At most two opens per requester and per provider are pending;
each reserves 16 MiB of retained payload for read and publication. Provider calls
share the existing sixteen host control-request slots with commands and input.

An already live canonical identity reuses its buffer without replacing edits or
its accepted version. A duplicate pending identity returns `busy`, including an
alias discovered by stat. The host rechecks handle capacity before publishing.
Completion reliably emits `resource.opened {job, buffer, revision, error}` with
null buffer/revision on failure, then terminal `job.changed`. A captured foreground
grant can present the result only while its pane, attachment and input context
remain valid; otherwise the document stays available in the buffer list. Job
acceptance does not extend that grant.

The requester may cancel an open, but cannot finish or update a host-owned read
job. Cancellation immediately prevents publication and releases read state;
there is no remote mutation to reconcile. Up to 64 retired in-flight request IDs
are remembered for late replies; other duplicate, foreign or out-of-order replies
are protocol failures. Provider failure, timeout or stop settles the open without
publishing partial text. No read creates a polling timer after completion.


### Native reload and conflict recovery

`:reload` reads a fresh, version-bound remote document. A clean document accepts
that text directly. Dirty or uncertain documents offer three native choices:

- Reload remote text, replacing the local text as one undoable edit.
- Keep local edits and use the remote version as the new saved baseline.
- Cancel, preserving local text and the previously accepted baseline.

Cancel is selected initially. Use the native choice keys and physical Enter to
accept a different decision; macros cannot authorize a remote reload.

Both adoption choices preserve earlier undo history. Undoing a replacement
restores the prior local text against the newly accepted remote baseline; undo
does not rewind the remote version. Keeping local edits adds no undo entry and
leaves the document dirty unless those edits equal the remote text.

The host prepares replacement text, history and display measurements before
offering the choice. Local and remote text must each fit 8 MiB; this operation
is independent of the 4 MiB side-by-side comparison limit. Changed document
revisions, provider generations or foreground context invalidate the captured
decision. Detach, cancellation and plugin stop preserve the original document.
Preparation reserves 16 MiB plus twice the local byte length, in addition to the
existing 16 MiB read or uncertain-write reservation. Capacity is checked before
preparation; canceled workers retain their charge until they actually finish.

An unknown prior write must be proven settled through the provider's reconciliation
contract before a new baseline can be accepted. An inspection snapshot alone
cannot provide that proof. Public `resource.rebind` remains conservative and
accepts only a known baseline; native reload makes divergent recovery an explicit
user decision. After restart, the provider must register the same identity again.
For the memory example, `:plugin.memory.rebind` performs that registration even
when its conservative reconciliation reports a conflict; `:reload` then offers
the native recovery choices.


### Remote conflict inspection

`resource.inspect {buffer, expected_revision, invocation}` requires `documents`
and `jobs`, an owned provider buffer at the expected text revision, and a live
foreground invocation for that document. Native `:diff-remote` uses the same
bounded read workflow. Inspection accepts a finite host-owned job, obtains fresh
`resource.stat` metadata and version-bound `resource.read` chunks, then opens a
read-only remote snapshot beside the editable document. Both sides must fit the
4 MiB comparison limit. A reused provider buffer never substitutes for this fresh
read.

The invoking command can return the associated job and finish. Before showing the
comparison, the host rechecks the captured pane, buffer, attachment, foreground
context and local text revision; changes can reject publication. Native inspection
uses a host-owned job even when the provider did not grant itself `jobs`. The
reliable completion event is `resource.inspected {job, buffer, revision, error}`,
identifying the generated read-only snapshot on success, followed by terminal
`job.changed`. Failure carries null buffer and revision.
Cancellation, provider failure or a changed remote version discards the pending
read without publishing partial text. Use `:diff-off` to close the comparison.

Inspection does not adopt remote text as the saved baseline, change the provider
binding/version, or clear an unknown write. It remains available to examine a
conflict while explicit reconciliation is refused. Even when the snapshot equals
an uncertain upload, comparison alone cannot prove that the previous remote
mutation has settled. Rebind still requires the settlement proof described below.

The provider receives reliable `resource.released {job}` on every read terminal
path, including cancellation, timeout, identity reuse and requester stop. This
notification belongs to the provider even when another application owns the job.
Release immutable cached data and cancel any still-running read for that job;
a release can overtake a queued resource handler or arrive before its response.
It does not establish settlement of a remote write. Failure to deliver the
notification stops the provider. The Python SDK dispatches these events through
its control worker so waiting command handlers cannot starve cache cleanup.

### Uploads and recovery

`buffer.save` uses the same host coordinator as native saves. It requires the
captured buffer revision, `documents` and `jobs`; native saves use host-owned jobs
even when the provider did not request the plugin-facing `jobs` capability.
Only one save or rebind per document is admitted. Native queued intents protect
close, discard, reload, quit and wait completion immediately. A text change before
host admission rejects that intent before trimming; after admission, the captured
snapshot is immutable and editing can continue.

The host sends `resource.write.begin {job, provider, key, expected_version, mode, bytes,
encoding}`, which returns `{kind: "write_started", value: {upload}}`. Begin and
all `resource.write.chunk {job, upload, offset, text}` calls must affect staging
only. Chunk replies are `{kind: "write_chunk", value: {offset}}`, acknowledging
the exact next UTF-8 byte offset. Document/chunk/encoded-frame limits match reads;
NUL text is refused before transport. Empty text still needs a final commit.

`resource.write.commit {job, upload, expected_version, mode}` carries the same
explicit mode as Begin. `conditional` requires the provider to enforce the original
remote precondition at the mutation, even if Begin checked it too.
`confirmed_best_effort` means the user approved a weaker overwrite: `expected_version`
still names the observed baseline, and the provider must compare it as closely as
possible before promoting the upload. That comparison does not establish an atomic
compare-and-swap and cannot exclude another writer between comparison and replacement.
Providers must reject an unsupported mode or a mode change between Begin and Commit.
Success is
`{kind: "write_committed", value: {version}}`. A confirmed non-commit is
`{kind: "write_rejected", value: {error}}`. `atomic_replace` independently declares
whether replacement is all-or-nothing for remote readers; neither declaration may
be inferred from a preflight stat followed by an unguarded upload. A failed
non-atomic replacement can leave partial content: return `outcome_unknown` unless
`write_rejected` can explicitly establish that the destination did not change.
Cancellation or a lost acknowledgement cannot establish non-commit.

For a provider without conditional writes, native save commands present a native
foreground overwrite confirmation. Only a physical Enter accepts it; generated
input cannot approve the overwrite. The host checks the captured document revision,
provider binding and foreground context again before admission. No provider call
is sent before approval, and cancelling leaves text, undo history and save hooks
untouched. Accepted approval permits one captured save, including its normal
trimming hooks; it does not authorize future saves or automatic retries. The prompt
describes the remaining external-edit race and whether replacement is atomic.
The captured preview reserves 16 MiB plus 512 KiB for bounded hook metadata.
Whitespace trimming is limited to 4,096 changes in a preview; a larger preview is
refused without editing the document. Trim it explicitly before retrying the save.
`:write!` does not bypass confirmation. Plugin-facing `buffer.save` continues to
refuse weak providers in this slice because it has no foreground approval field.

A confirmed save adopts only the uploaded text as the saved baseline. Later edits
stay dirty and undoing back to the uploaded snapshot becomes clean. Reliable
`resource.saved {job, buffer, revision, error}` identifies the uploaded revision on
success, with null revision on failure, followed by terminal `job.changed`. The
live buffer revision can be newer. Save-and-close runs only after accepted success,
when that captured buffer is still clean in the same pane, attachment and foreground
context. Acceptance alone never closes or completes a wait. The synchronous bundled
client SaveBuffer endpoint refuses provider documents rather than acknowledging
pending durability. Remote resource keys never enter local write, Git or LSP paths.

Before Commit is queued, cancellation or deadline requests
`resource.write.abort {job, upload?}` and keeps protection until
`{kind: "write_aborted", value: {}}`. Abort has a two-second deadline; unresponsive
providers are stopped. Providers must remember aborted job identities sufficiently
to prevent a late Begin from reviving staging, including when `upload` is null.
Abort is staging cleanup and cannot undo an already committed write. Ordinary
control calls have ten-second deadlines and the complete save job has sixty seconds.
A stopped requester leaves bounded cleanup work protected until it settles.

Once Commit entered the outbound queue, cancellation, response loss, timeout,
malformed acknowledgement or provider stop can mean `outcome_unknown`. Even a
`write_rejected` carrying that error code retains uncertainty. Runyte preserves the
uploaded snapshot and dirty protection, never retries automatically, and ignores
late acknowledgements as editor mutations. Explicit discard cannot clear this
uncertainty or permit an unverified retry.

`resource.rebind {buffer, expected_revision}` binds a restarted provider only after
reading version-bound content. Matching the accepted baseline preserves local edits;
matching an uncertain uploaded snapshot establishes that snapshot as saved. Divergent
content returns `conflict` without changing either text or baseline. Rebind guards
baseline-changing operations while allowing live editing and rechecks the baseline
epoch before adoption.

For an uncertain write, an ordinary stat is insufficient: its commit could still
be in flight. The host sends `resource.reconcile {job, provider, key, previous_write}`.
The provider must return `{kind: "reconciled", value: {metadata, previous_write}}`
only after proving that exact previous mutation has settled and cannot commit later.
The following read chunks pin that metadata version. If the transport cannot
establish settlement, it must return `outcome_unknown`; reconnecting and reading old
bytes is not proof. The memory example serializes commit and reconciliation under
one lock and has no mutation that can outlive its process. A network provider must
supply its own honest settlement mechanism.

Unknown uploads retain a 16 MiB reservation under the originally charged configured
requester, across provider/requester restarts. That reservation covers the captured
8 MiB text and its bounded recovery read, so full retained quotas cannot prevent
recovery. Reconciliation compares directly against the two known baselines without
allocating another text copy. Success or explicit document retirement releases the
reservation; failed/cancelled rebind retains it. This accounting is independent from
whether the provider and requesting application are the same process.


## SFTP browser and editor

The runnable `sftp.py` application uses `remote_provider.py`,
`remote_application.py` and `sftp_transport.py` beside the shared `application.py`
client and `transport.py` worker. The shared `remote_download.py`,
`remote_operations.py` and `remote_status.py` modules provide transfer and
namespace-operation workflows. SSH stays in the
application process; Runyte gains no SSH library dependency. Install Paramiko in
a separate Python environment and use that environment's interpreter:

```sh
python3 -m venv /path/to/sftp-env
/path/to/sftp-env/bin/python -m pip install paramiko
```

Create a JSON connection profile outside the repository, for example
`/path/to/sftp-profile.json`:

```json
{
  "alias": "development",
  "host": "development.example.org",
  "port": 22,
  "username": "editor",
  "root": "/srv/project",
  "known_hosts": "/path/to/verified_known_hosts",
  "identity_files": ["/path/to/ssh_identity"],
  "allow_agent": false
}
```

The root is an absolute remote directory. Relative command paths resolve below
it, and paths outside it are refused. These canonical path checks assume a
trusted server namespace; SFTP cannot hold a directory-relative identity across
all operations. Use a server-side chroot when confinement is required. The
profile accepts only the documented
fields; `port` defaults to 22, `identity_files` to an empty list and `allow_agent`
to false. Supply an explicit identity file or enable the existing SSH agent.
There is no password field, interactive password prompt, automatic credential
search, or plaintext credential persistence. To use an encrypted identity, load
it into the SSH agent and set `allow_agent` to true. Connection errors never
include library exception details or credential paths in application messages.

Populate `known_hosts` through a trusted host-key verification process before
starting the application. Unknown or changed host keys are refused; the example
does not accept keys automatically. It does not invoke a shell or interpret
`~/.ssh/config`. Use absolute local paths in the profile.

```yaml
plugins:
  - id: sftp
    enabled: true
    api: runyte-experimental-2
    executable: /path/to/sftp-env/bin/python
    args:
      - /path/to/runyte/docs/plugins/sftp.py
      - --config
      - /path/to/sftp-profile.json
    capabilities: [views, providers, documents, jobs, filesystem, interaction]
```

For another configured plugin ID, also pass `--plugin-id` with that ID. Each
process has one profile and registers provider `remote`. The connection identity
includes the endpoint and remote root, while labels use the profile alias;
credential paths are not resource keys.

Run `:plugin.sftp.browse .`. Enter browses the selected directory or opens a
regular UTF-8 file as a normal editable provider document. `Tab` exposes the same
registered actions as the palette: `:plugin.sftp.parent` and
`:plugin.sftp.refresh`. Row identities survive reorder and refresh, while actions
from stale view revisions are refused. Symbolic links are displayed but not
followed. `:plugin.sftp.open "notes/猫 notes.md"` opens a path directly. The browser
is bounded to 1,024 entries and a 900 KiB encoded model; larger directories are
refused without replacing the previous view. Network work runs outside the SDK
reader, has a finite transport deadline, and does not poll while idle. Overlapping
browser commands return `busy` instead of waiting behind a network operation.

Edit with normal Runyte commands, then use `:write`. Every SFTP save presents the
native overwrite confirmation. The provider compares the previously observed
content version before promoting a temporary upload with the server's atomic
POSIX-rename extension. This detects observed conflicts but is **not an atomic
compare-and-swap**: a remote writer can still change the file between comparison
and replacement. The confirmation describes that remaining race. A server
without the required replacement extension cannot complete the save; the adapter
does not fall back to deleting the destination first. There is no plugin-facing
save command that bypasses native approval, and `:write!` does not bypass it.

`:write-quit` and `:write-buffer-close` close only after confirmed clean success
in the original foreground context. Edits made during an upload remain dirty.
`:diff-remote` or `:plugin.sftp.inspect` compares a fresh remote snapshot without
changing local text or its saved baseline; each side is limited to 4 MiB.
`:plugin.sftp.rebind` explicitly reconciles a document after provider restart or
an uncertain upload. An uncertain remote write keeps local data dirty and cannot
be retried blindly; if settlement cannot be proved, rebind remains refused.
Remote documents are limited to 8 MiB of UTF-8 and never acquire a local file path.
Binary downloads and disk-file uploads use the confirmed workflows below;
remote mkdir/rename/delete commands have a separate confirmation workflow.
The automated SFTP fixture uses temporary local credentials and a loopback server,
and never contacts a live account.


## FTP and FTPS browser and editor

The runnable `ftp.py` application reuses the same `remote_application.py` browser,
`remote_provider.py` engine, `application.py` client and `transport.py` worker
as SFTP. Its `ftp_transport.py` adapter uses Python 3.10+ standard-library `ftplib` and `ssl`;
Paramiko is not imported or required. Configure explicit TLS with a JSON profile
outside the repository, for example `/path/to/ftps-profile.json`:

```json
{
  "transport": "ftps",
  "alias": "development",
  "host": "development.example.org",
  "port": 21,
  "username": "editor",
  "root": "/project",
  "password_file": "/path/to/private/ftp-password",
  "ca_file": "/path/to/trusted-ca.pem"
}
```

`transport` defaults to `ftps` and `port` to 21. FTPS uses explicit TLS on the
control connection and requires encrypted data connections (`PROT P`).
Certificate chains and hostnames are verified; omit `ca_file` to use the system
trust store or provide an absolute path to your trusted CA file. TLS failures
never fall back to FTP. Implicit FTPS is not implemented.

Store the password as one UTF-8 line in the absolute `password_file`, optionally
ending with LF or CRLF. The file must be a regular file owned by the current user,
with no group or other access (for example mode `0600`), and is read without
following symbolic links.
Its contents are bounded to 4,096 bytes. The profile contains a credential-file
reference; it has no raw password field. Passwords are not sent through editor
prompts, configuration arguments, resource keys, views, notifications or logs.
The application reads the private file for authentication and does not create a
credential cache. Keep this file outside tracked workspace content.

To use plain FTP, explicitly set `"transport": "ftp"` and remove `ca_file`,
which is only valid for FTPS. The application name,
command descriptions and browser title display **FTP (unencrypted)**. FTP sends
credentials and document data without transport encryption; it is a separate
connection choice, never a compatibility fallback. The opaque connection identity
includes the transport, so changing FTP to FTPS does not silently reuse a document
binding from the other connection.

```yaml
plugins:
  - id: ftp
    enabled: true
    api: runyte-experimental-2
    executable: /usr/bin/python3
    args:
      - /path/to/runyte/docs/plugins/ftp.py
      - --config
      - /path/to/ftps-profile.json
    capabilities: [views, providers, documents, jobs, filesystem, interaction]
```

Pass `--plugin-id` as well when the configured ID differs from `ftp`. Run
`:plugin.ftp.browse .`; Enter opens a selected directory or regular UTF-8 file.
`:plugin.ftp.parent`, `:plugin.ftp.refresh`, `:plugin.ftp.open "notes/猫 notes.md"`,
`:plugin.ftp.inspect` and `:plugin.ftp.rebind` have the same captured-context and
revision behavior as SFTP. The browser requires structured server metadata and
does not parse presentation-oriented `LIST` output. Directory listing is bounded
to 1,024 entries and a 900 KiB encoded view; documents are bounded to 8 MiB of
UTF-8. Remote path checks assume a trusted server namespace; enforce confinement
with the server account's root or jail.

Use native `:write` to save. Every FTP/FTPS save requires foreground confirmation:
this adapter advertises **neither conditional writes nor atomic replacement**.
It compares the observed content version before remote promotion, detecting
observed changes while retaining the race with a concurrent remote writer. An
upload or rename failure may leave partial remote state; the confirmation names
that limitation. The adapter never deletes the destination as a rename fallback
and does not automatically retry failed mutations. Remote staging permissions
follow server policy; the adapter does not promise private staging files.

Save-and-close waits for confirmed clean success, and edits during upload remain
dirty. `:diff-remote` inspects remote changes without altering the local saved
baseline. After a disconnect during promotion, local text remains protected with
an unknown write outcome. Explicit rebind succeeds only when the provider can
prove the previous write has settled; reconnecting alone is not proof. If a
server's rename behavior cannot complete replacement, the save is refused without
weakening these guarantees. Binary downloads, disk-file uploads and remote
mutation commands use the confirmed workflows below. Automated fixtures use isolated loopback FTP/FTPS servers and
temporary credentials and certificates, never a live account.


## Binary downloads and confirmed publication

Binary bytes stay in the application process and local staging files; they are
not encoded into extension messages or inserted into provider text buffers. The
host issues a private destination, verifies and freezes the completed download,
and uses the existing native filesystem confirmation to publish a new local file.
The initial limit is 8 MiB per download, including arbitrary non-text formats and
empty files. Larger downloads are refused.

In either remote browser, select one regular file and run
`:plugin.sftp.download` or `:plugin.ftp.download` from the palette or `Tab` actions.
The native prompt asks for a new workspace-relative local destination. The
application returns a finite transfer job immediately, then streams the file and
prepares a plan outside the command handler. A browser status row reads
`Download ready` and names the next command. Run `:plugin.sftp.confirm-download`
or `:plugin.ftp.confirm-download` to present native filesystem confirmation using
a fresh invocation; publication always requires this separate review step. `cancel-download` cancels the pending
prompt, transfer or unpresented plan. Each application retains one download flow
at a time; the transfer job stays running until confirmation is presented and
expires after 60 seconds if the flow is not completed. Both examples require
`filesystem` and `interaction` alongside their existing capabilities.

The shared `remote_download.py` workflow reports only phase changes to
`remote_status.py`. Status publication preserves file row identities, coalesces
updates in one bounded worker, and rechecks the captured browser view and phase
generation. A busy view gets at most two short retries during that pending update;
there is no status polling or idle timer. Closing a browser never recreates it in
the background.

`staging.create {job, bytes}` requires an owned running transfer job and an exact
byte count from 0 through 8,388,608. It returns `{staging, path}`: an opaque handle
and the path of an exclusively created private file. Write only that issued path;
it is temporary runtime state, not a document identity or a durable destination.
The application must stream within the declared limit, close its writer, and
compute the SHA-256 digest of the completed bytes. The host validates its limits
when sealing; an enabled unsandboxed process still has its ordinary filesystem
permissions while it writes.

Obtain a destination directory through `filesystem.list`, then call:

```json
{
  "type": "request",
  "id": "p:702",
  "method": "staging.prepare",
  "params": {
    "staging": "s:g:1",
    "directory": "d:g:1",
    "expected_revision": "d:1",
    "destination": "download.bin",
    "sha256": "8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8"
  }
}
```

`destination` is workspace-relative, matching existing filesystem intent paths;
the directory handle identifies the confirmation snapshot scope. It must remain
within the workspace. An existing destination, stale directory revision, foreign
handle,
changed staging identity, wrong byte count or digest is refused before any user
file is written. The SHA-256 value is exactly 64 lowercase hexadecimal characters.
No arbitrary source path is accepted in this request.

Preparation runs outside the editor loop. It copies the completed bytes into a
separate host-owned file and checks their size and digest. The writable staging
handle is consumed only after successful preparation, which returns the existing
`{plan, operations}` filesystem-plan result. A writer that retained the original
file descriptor cannot subsequently change the frozen copy or the published
file. Path renaming or changing file permissions alone would not provide this
separation.

The transfer job must remain running through preparation and until
`filesystem.apply {plan, invocation}` successfully presents native confirmation.
A finish, cancellation, deadline or explicit staging close that overtakes pending
preparation invalidates its result. Finishing or cancelling the transfer before
confirmation also retires its unpresented plan. Preparation itself grants no
foreground authority: application code must supply a fresh still-pending command
or accepted native-input invocation when presenting the plan.

Once `filesystem.apply` presents confirmation, ownership moves to that native
surface and the original transfer job can finish. Pressing Enter starts the
existing asynchronous filesystem apply job; acceptance is not completion. Escape
cancels without publishing the file. Destination changes are revalidated before
application. Pending publication uses the same document mutation barrier,
owner-stop reconciliation and failure reporting as other application filesystem
plans. No download automatically opens or executes its contents.

`staging.close {staging}` releases an unprepared handle. Handles and prepared
sources belong to one configured owner and connection generation; guessed or
foreign handles grant no access. The host allows two writable staging files per
owner, reserves 64 KiB for each
issued handle and 24 MiB while sealing, and charges prepared plans through the
existing filesystem budget. These reservations bound retained sealing work
alongside the shared application budget. Cancellation and process cleanup release unaccepted
staging, while an already accepted native filesystem operation retains its frozen
source until that operation settles.


## Binary uploads from workspace disk files

In either remote browser, run `:plugin.sftp.upload` or `:plugin.ftp.upload`.
The native form asks for a workspace-relative source file and a destination
relative to the displayed remote directory. This uploads the file's disk bytes;
unsaved editor text is separate. Files must be ordinary files within the workspace,
without symbolic-link traversal, and fit 8 MiB. Empty files, NUL bytes and non-UTF-8
data are accepted. Binary data stays in the application process and is never sent
through the editor's text or extension-message APIs.

Preparation freezes the source bytes and reads the remote destination's current
content hash or proves its absence. It does not upload data. When the browser
reports `Upload ready`, run `:plugin.sftp.confirm-upload` or
`:plugin.ftp.confirm-upload`. A fresh native confirmation names the source,
destination, byte count and transport limitations. Unrenderable or excessive
labels are refused instead of shortened. Changing the source file after preparation
does not alter the frozen bytes that confirmation authorizes.

Only an accepted confirmation starts the remote upload. The transport rechecks
the prepared destination, writes a temporary remote staging file, then promotes
it once. An observed remote content change or newly occupied destination refuses
the operation. SFTP new-file promotion uses ordinary rename; existing-file
replacement requires the server's POSIX-rename extension. FTP/FTPS rename may
overwrite a destination created after its final check and is not guaranteed atomic.
FTP staging permissions follow server policy. Neither transport claims atomic
compare-and-swap, removes a destination as a fallback, or retries promotion.

`cancel-upload` cancels a prompt, prepared upload or pending transfer. Each
application admits one upload flow; its finite job includes preparation and human
review, with a 60-second deadline. Workers retain their admission until they
actually finish. Progress reports bounded increments and reaches completion only
after an acknowledged promotion. Phase changes update the existing browser without
an idle poll; accepted work can finish while detached.

A failure after promotion may have an unknown outcome. That flow retains its
source snapshot and destination report in the current plugin process, blocks another upload, and cannot be
cleared by cancellation. Inspect the remote destination and explicitly restart
the plugin before preparing a new attempt; reconnecting or observing matching
bytes alone is not proof that the original operation has settled. There is no
automatic retry. Uploads do not modify an open provider document, its local text,
or its saved baseline; use ordinary provider reload or save conflict handling
when the uploaded destination is also open in the editor.

## Confirmed remote directory operations

Both reference browsers use the existing `ui.prompt`, `ui.confirm` and finite-job
APIs for remote mkdir, rename and permanent delete. No additional editor method
or transport library is needed. The application owns remote IO and preconditions;
Runyte owns the input surface and sends the accepted callback only after native
confirmation. A trusted application already has ordinary OS/network permissions:
this is an application workflow, not a sandbox for its network traffic.

Use these actions in an SFTP or FTP/FTPS browser (replace `sftp` with `ftp` for the
second adapter):

- `:plugin.sftp.mkdir` prompts for a destination relative to the displayed directory.
- `:plugin.sftp.rename` prompts for a destination relative to the selected entry's parent.
- `:plugin.sftp.delete` prepares deletion of the selected entry.
- `:plugin.sftp.confirm-operation` shows the prepared operation in native confirmation.
- `:plugin.sftp.cancel-operation` cancels the pending prompt, inspection or operation.

Preparation returns a finite job immediately and inspects remote state in a
bounded worker. A retained browser row reports `Remote operation ready` and names
the confirmation command. Run it with a fresh invocation, review the quoted exact
paths and transport warning, then press Enter to apply or Escape to cancel.
Neither preparing an operation nor invoking the confirmation command performs
the mutation. Confirmation cancellation changes no remote entry. Completion
leaves a visible result and a refresh action; a closed browser is never reopened
by background work. Download and operation status rows preserve one another.

Each application retains one operation and one worker. Its job lasts at most
60 seconds, including preparation and human review. Each network operation uses
the adapter's eight-second caller deadline. Namespace mutations and document
replacements share one mutation slot, retained until the actual worker exits,
even if its caller has already timed out. Reads remain independently bounded.
Cancellation before the mutation gate prevents the mutation; a remote command
already sent may finish despite cancellation.

The initial operations handle one regular file or empty directory as reported by
the server. File inspection is limited to 8 MiB; directory metadata keeps the
existing 1,024-entry/4 MiB bounds. Nonempty directories, recursive deletion,
configured-root mutation, parent traversal and existing destinations are refused.
The native confirmation currently accepts 160 UTF-8 bytes each for title and
message. If exact quoted paths, the connection label and mandatory warnings do
not fit, preparation is refused; they are never truncated to obtain approval.

Prepared values are immutable and bound to the connection identity. Apply resolves
the paths again, compares source metadata and content hash, and rechecks destination
absence before sending the mutation. These are best-effort checks, not
compare-and-swap. SFTP uses ordinary rename and never the overwrite extension for
a namespace rename. FTP/FTPS has no portable no-replace rename guarantee: a target
created after the check may be overwritten, and the native warning says so.
The adapter never deletes a target as a rename fallback. Deletion is permanent;
there is no filesystem undo. SFTP rejects source symbolic links; FTP relies on
server-reported types and cannot identify a link reported as an ordinary file.
Use the server account's root/jail for confinement against namespace races.

A dropped reply, timeout or unproved failure after mutation submission produces
`outcome_unknown`. The status tells the user to inspect remote state and not
retry. The application never automatically repeats an operation, and reconnecting
or reading current metadata is not proof that an earlier command cannot still
finish. If cancellation reaches the host before a successful acknowledgement can
finish its job, the application records a conservative unknown terminal outcome
instead of leaving the job stuck cancelling.

Remote namespace changes never retarget, close or replace editor buffers. An open
provider document keeps its original resource identity, saved baseline, undo and
local edits after remote rename or deletion. A later save to a missing or changed
source fails its existing preconditions, preserving unsaved text; it does not
recreate the removed path. Open the new resource explicitly after a rename.
Document versions use content hashes: replacing a path with identical bytes is
indistinguishable from unchanged content, so these checks do not prove that the
server retained the same filesystem object.
The isolated server fixtures cover cancellation, changed sources, collisions,
empty-directory limits, uncertain replies and serialization with document saves.

## Source subscriptions

`event.subscribe` takes `sources`, a nonempty array of filters. Use
`{"kind":"buffers"}` for existing and newly opened buffers, or explicit issued
handles such as `{"kind":"buffer","buffer":"…"}` and
`{"kind":"pane","pane":"…"}`. These and `{"kind":"attachment"}` require
`workspace`. Owned `view` and `job` handles require `views` and `jobs` respectively.
Foreign or closed handles cannot create a subscription. Filters never authorize
text access or grant another application's owned view or job handles.
Workspace buffer discovery includes metadata for generated application buffers,
just like `buffer.list`.

The response contains `subscription`, `sequence` and `sources`, captured in the
same host turn. Each source entry contains a concrete `source`, opaque observation
`revision` and typed `state`. Buffer metadata includes text revision, accepted
saved revision, dirty/read-only flags, scalar length and a bounded display label.
No text is copied. Pane metadata identifies its displayed buffer (null for a
terminal) and selection revision. View metadata contains its model revision and
optional query metadata;
job metadata contains state/progress; attachment metadata contains attached state
and generation. Read text, selections and models through their existing explicit
APIs when needed. An accepted save of an unchanged baseline need not emit a new
state; the saved revision describes the baseline, not a count of save commands.

`event.changed` carries `data: {subscription, sources, coalesced}`. Buffer-open,
saved-baseline, job-state and attachment transitions use reliable delivery.
Ordinary text invalidations, selection/model revisions and progress can coalesce:
only the latest metadata survives, and `coalesced` reports replaced pending
observations. Closing a source produces reliable `event.closed` with a
`{"kind":"closed"}` state. The outer `sequence` increases in connection delivery
order, including other application events; source revisions describe snapshots.
Neither is an edit log, and cross-application ordering is unspecified.

The host admits at most 32 subscriptions and 256 subscription/source pairs per
application, counting a source watched twice twice. Each subscription retains at
most 64 pending changed sources. Exceeding the pending table or discovery limit
emits reliable `event.resync_required` and suspends that subscription. Call
`event.resync` with its handle to obtain a fresh baseline; an oversized baseline
is refused without replacing the old subscription. There is no replay. A bounded
reliable queue preserves lifecycle transitions, responses and accepted actions;
an owner that cannot accept them is stopped. Eight outgoing message slots and
512 KiB remain reserved for reliable control. A drain notification flushes the
last coalesced state without waiting for input or starting a timer.

`event.unsubscribe` is idempotent. Previously queued observations precede its
acknowledgement; no later event names that subscription. A new subscription gets
a new handle and baseline. Subscriptions disappear when their owner stops.
Mutation responses precede resulting invalidations. With no subscriptions there
is no subscription scan, worker or timer. New-buffer discovery caches membership
and revisits it only after buffers open or close.

The Python client offers ordered callbacks and queues baselines before subsequent
events, including events received before `subscribe` returns:

```python
def observe(event, sequence, data):
    # event.baseline is an SDK callback name; other names are host events.
    # Schedule expensive work elsewhere and return promptly.
    pass

baseline = app.subscribe([{'kind': 'buffers'}], observe)
# After event.resync_required, explicitly request the new baseline:
app.resync(baseline['subscription'])
app.unsubscribe(baseline['subscription'])
```

Callbacks run on one bounded observation worker, separately from command,
provider and cancellation handlers. The client refuses a full 32-callback queue
instead of silently losing observations. Callbacks already queued before an
unsubscribe acknowledgement may still finish locally. View query/viewport/action observations and helper-exit sources are
still pending alongside their corresponding later application features.

Authors using generic `app.request('event.subscribe', sources=...)` receive updates
through `on_observation(event, sequence, data)`, which defaults to forwarding to
`on_event(event, data)`. Use the convenience helper when baseline and update
processing must run through the same ordered callback lane.

## Managed helpers

`helper.py` is a runnable, network-free controller with capabilities `processes`,
`views` and `interaction`. Configure it like the jobs example with ID `helper`,
then run `:plugin.helper.open`. Tab offers Send, Flood, EOF and Close. Its
checked-in `echo_helper.py` backend demonstrates binary pipe separation, retained
output eviction, natural EOF exit and explicit stop. The native view renders only
a bounded output tail and groups pending output observations; it has no idle
refresh timer. Closing its view retains the helper; Close stops and releases it,
and stopping the plugin always cleans up its managed helper group.

`process.start {label,executable,args?,cwd?,capture_stderr?}` runs an argument
vector without shell interpolation. A successful reply arrives only after
spawn and returns an opaque `process` handle plus the metadata described below.
The safe display label is 1–160 UTF-8 bytes. Executable and optional working
directory are at most 4,096 bytes, and at most 64 arguments each contain at most
4,096 bytes and 64 KiB in aggregate. Null bytes are refused. The default working
directory is the workspace root; an explicit directory must resolve to an
existing directory inside it. Path resolution and spawn run off the editor loop.
Startup has a five-second deadline. A late spawn cannot issue a successful
handle; it is immediately cleaned up, while its reserved ownership and quota
remain until actual completion. The helper inherits ordinary user permissions
and environment. This API provides
ownership and data bounds, not an OS security sandbox.

`process.get {process}` returns `{process,label,state,stdout,stderr,
capture_stderr,stdin_closed,exit_code,signal,output_truncated}`. State is `running`,
`closing` or `exited`. Each stream exposes `{start,end}` monotonic byte offsets
for its retained ring. Exit code and signal are nullable. A naturally exited
handle retains readable output and continues to count against quotas until
explicitly closed. Nothing silently evicts another helper to make room.

`process.read {process,stream,offset,limit}` returns
`{process,stream,offset,data,next,eof}`. Streams are `stdout` and `stderr`; `data`
is standard base64, and `limit` is 1–65,536 decoded bytes. Reading before retained
`start` returns `stale`; reading beyond `end` returns `invalid_argument`. An empty
chunk with `eof:false` is valid. `eof` means this captured stream has ended and the
cursor reached its retained end; inspect `output_truncated` before assuming it
contains every byte the helper produced. Subscribe to process metadata instead
of repeatedly polling an empty stream. Stderr defaults to a null sink; reading
it returns `unsupported` unless capture was explicitly enabled. The host never
copies raw helper output, arguments or stdin into diagnostic logs.

`process.write {process,data,eof?}` accepts at most 65,536 decoded base64 bytes
and one outstanding write per helper. The deferred result
`{process,written,stdin_closed}` acknowledges bytes written to the OS pipe; it
does not claim the child consumed them or performed an action. `eof:true` closes
stdin after those bytes. A write that cannot finish within five seconds stops
the helper and reports `outcome_unknown`; partial delivery is never retried.

`process.close {process}` requests group cleanup and returns `{}` only after
actual reap and handle removal. Repeated closes join the cleanup; closing an
unknown handle is idempotent. Running/closing state and stdin closure are reliable
observations through source `{kind:"process",process}`; stdout/stderr bounds are
coalescible invalidations, without raw bytes. Close acknowledgements precede the
resulting source-closed observation. `get` can report `closing` while a deferred
close waits. The final drain is at most 64 KiB across both pipes and waits only a
bounded time for escaped pipe holders; `output_truncated` reports an incomplete
drain. This final output travels in the reserved terminal event, so filling the
ordinary output queue cannot prevent reap or terminal-result delivery.

The handshake advertises `process_handles`, `process_io_bytes`,
`process_output_bytes` and `process_write_seconds`.

Four retained helpers per configured plugin identity and 32 across the host
include pending spawns and cleanup from stopped generations. Each reserves 2 MiB
of retained payload budget. Output retention is 1 MiB for stdout alone, or 512 KiB
per stream when stderr capture is enabled. Every helper has three ordinary event
slots and one reserved terminal slot. Pending start/write/close requests share
the owner's sixteen-request bound and temporarily protect persistent-host idle
retirement. An idle running helper alone does not protect retirement.

Linux/macOS cleanup kills the owned process group while its leader remains
unreaped, then reaps it. The same anchoring rule covers natural exit, explicit
close, owner stop and abandoned spawn tasks. Descendants that deliberately escape
the group are outside this guarantee. Exit handling uses child-exit notifications,
with no per-helper polling timer. The Python SDK's `start_process`, `read_process`
and `write_process` helpers preserve these bounds; reads return decoded bytes,
and invalid write acknowledgements produce `outcome_unknown` without replay.

## Notifications and native handoffs

`handoffs.py` demonstrates these operations. Configure it like the jobs example,
with ID `handoffs` and capabilities `notifications`, `terminals` and `external`.
Run `:plugin.handoffs.notify`, then `:notifications` to read the retained message.
`:plugin.handoffs.terminal` opens the configured environment's interactive shell
in a native terminal session. `:plugin.handoffs.browser https://example.org/`
requests a system-browser handoff, and `:plugin.handoffs.file report.pdf` requests
the system handler for an existing workspace file. These are explicit actions;
the example never opens a browser during registration or in its conformance tests.

`notification.publish {severity,title,body}` needs capability `notifications`.
Severity is `info`, `warning` or `error`. The title is 1–160 UTF-8 bytes without
controls; body is at most 4,096 UTF-8 bytes and 64 lines, with only newline and tab
allowed as controls. Empty body is valid. The host labels the source with the
configured plugin identity; the plugin cannot choose another source. Publication
works while detached and does not replace action feedback, move focus or open a
buffer. It wakes presentation once so an idle attached frontend sees its unread
indicator; multiple accepted messages rebuild retained notification documents
once at the next presentation checkpoint. There is no notification refresh timer.

Publication accepts at most one message per 500 milliseconds per plugin; a faster
call returns `busy`, without extending the interval. Consecutive identical messages
use the native occurrence counter. Native notification-history count and byte
limits apply, and records remain available after the plugin stops. Notifications
alone do not protect idle retirement. The SDK helper is `publish_notification`.

`terminal.open {invocation,label,executable,args?,cwd?}` needs capability
`terminals` and a current foreground invocation. Argument, label and cwd bounds
match `process.start`; stdin/stdout use a PTY instead of the helper pipe API.
Arguments retain their exact boundaries and are never joined into a shell command.
Cwd must resolve to an existing workspace-contained directory. Preparation and
spawn run on a bounded worker. The host rechecks foreground context before
installing the terminal and returns `{terminal}` as an opaque handoff receipt.
The receipt does not grant plugin terminal-input or terminal-close operations.

Early PTY output waits for native installation. Cancellation, owner stop or a
stale foreground context before installation cancels the reader gate, signals the
still-owned process group and retains accounting through actual cleanup. Native
installation transfers ownership to the editor: the terminal session survives
plugin stop and follows normal terminal navigation, close and persistent-session
retention rules. Eight unpublished terminals globally reserve 4 MiB each; stale
cleanup continues to occupy its reservation until settled. The SDK helper is
`open_terminal`; the example passes an executable and argument list directly.

`external.open {invocation,target}` needs capability `external` and consumes one
foreground invocation when launch is admitted. Target is either
`{kind:"url",url}` or `{kind:"file",path}`. URLs are at most 4,096 UTF-8 bytes,
require an explicit HTTP/HTTPS authority and host, and reject credentials,
controls, whitespace and backslash ambiguity. Encode intended URL spaces as
`%20`. File targets resolve to an existing regular file inside the workspace;
the canonical absolute path becomes one argument. The fixed handler is
`xdg-open` on Linux and `/usr/bin/open` on macOS. Other schemes, custom handler
commands and arbitrary shell execution are outside this operation.

Validation runs off the editor loop and is followed by a fresh foreground check.
Successful `{}` means the OS accepted the handler spawn, not that a browser opened,
a login succeeded or playback started. Handler exit status cannot retroactively
turn this receipt into a delivery guarantee. Handler startup waits at most five
seconds; the host also applies an eight-second overall handoff deadline. After
irreversible launch admission, a timeout is `outcome_unknown`, and the bounded
system-opener owner still handles late spawn/reap. Never automatically retry it.
At most sixteen system-opener records exist across the process; handlers have
independent stdio/process groups and continue independently of plugin stop.
Targets and URL query data never appear in host diagnostic messages.

Each active plugin generation has at most one pending handoff; retired
generations may retain separate cleanup within the shared worker and terminal
limits. Preparation has a five-second
response deadline; actual worker and unpublished-terminal cleanup retain shared
local-work permits and payload charges after a timeout or owner stop. External
preparation reserves 128 KiB. Foreground context is checked at admission and at
the asynchronous handoff boundary; missing attachment, changed focus, stale
invocations and cross-owner tokens cannot take presentation. An external launch
consumes its invocation, preventing repeated windows from the same action.
The SDK's `open_url` and `open_file_externally` preserve explicit target kinds and
never replay failures. These handoffs complement managed helpers; they do not
provide a browser widget, decoder, credential store or inline video surface.

## Continuing activity

Capability `activity` lets a deliberately continuing service protect its workspace
while it works. Registration, an idle helper and a retained view alone do not
protect idle retirement. Each owner may retain two activity leases, counting
both active work and cancellation awaiting cleanup. Each lease costs 1 KiB of
the existing retained-payload allowance. Leases are connection-generation handles;
never save one in configuration or workspace state.

`activity.acquire {title,duration_seconds?}` returns
`{lease,title,state:"active",duration_seconds}`. The immutable title is 1–160 UTF-8
bytes with no controls. Duration is an integer from 1 through 600 seconds and
defaults to 600. Acquisition does not need a frontend or captured command grant.
`activity.renew {lease,duration_seconds?}` sets a still-active lease's deadline
relative to that request; it cannot revive an expired or cancelling lease.
`activity.get {lease}` returns its current metadata. Duration describes the most
recent grant, not a ticking remaining-time counter.

`activity.cancel {lease}` changes active work to `cancelling` and sends the owner
one reliable `activity.cancel_requested {lease,reason:"cancelled"}` event after
the response. Expiry follows the same cleanup path with `reason:"expired"`.
Renewal cannot beat an already-passed deadline merely because its timer event is
still queued. Repeated cancellation does not extend the cleanup grace period.

The owner must stop its continuing work and acknowledge completed cleanup with
`activity.release {lease}` within two seconds. Release returns an empty result;
it is also the ordinary completion operation when work stops voluntarily.
Repeated release of an issued handle in the same generation is harmless.
Do not release before cleanup has actually finished. A missed acknowledgement
stops the owner through ordinary plugin teardown, including managed helper
cleanup. Native terminals and accepted external-handler handoffs keep their
independent ownership. One cancelled lease can therefore stop other work owned
by an unresponsive plugin.

Both active leases and the bounded cancellation grace protect normal shutdown
and idle retirement. Persistent-session health reports owner, title and state;
`:service-health` shows the same continuing work. Detach retains it. Explicit
forced persistent-session stop remains available; `:q!` only overrides the
ordinary unsaved-document guard and does not silently abandon plugin activity.
Use `:plugins` to stop the owner before quitting in either deployment mode.
Plugin stop or host restart invalidates every lease.

The SDK exposes `acquire_activity`, `renew_activity`, `get_activity`,
`cancel_activity` and `release_activity`. It starts no renewal thread and never
replays a failed renewal. Decide explicitly whether playback or service work is
still wanted before renewing. `activity.cancel_requested` uses the separate
bounded cancellation worker, so ordinary command handlers waiting for replies
cannot starve cleanup. Keep that callback independent of locks held by command
handlers; signal cancellation promptly and acknowledge only after the work stops.
The host uses its existing one-shot deadline scheduler, with no activity timer
when no lease or cleanup grace is present.

## Local media controller

`media.py` is a standard-library Python application backed by an installed mpv
(0.37 or later). Runyte presents the playlist and current-item dashboard; mpv
owns decoding, audio output and its external video window. The example accepts
ordinary audio/video files inside the workspace. It refuses URLs and authored
playlist files; Spotify and YouTube use separate service adapters described in
the [media service guide](media-services.md).

```yaml
plugins:
  - id: media
    enabled: true
    api: runyte-experimental-2
    executable: /usr/bin/python3
    args: [/path/to/runyte/docs/plugins/media.py]
    capabilities: [views, processes, activity, jobs]
```

Use `:plugin.media.open music/first.wav` to replace the playlist and play a file,
or `:plugin.media.add music/second.wav` to append one. Paths are workspace-relative;
quote spaces using the ordinary command syntax. `:plugin.media.show` returns to
the retained view. Select an item and use its primary action to play it. The
view also exposes `play`, `pause`, `seek`, `next`, `previous` and `stop-playback`; for
example, `:plugin.media.seek -10` seeks ten seconds backwards. Controls are
serialized, and each finite control job waits for the backend acknowledgement.
Next/previous preserve pause. Playing the currently paused row resumes at its
existing position; selecting a different row starts that item.
There are at most 64 entries and 16 KiB of combined canonical path text.

Playback acquires an activity lease before starting or resuming the player. It
renews only while playback is wanted, releases after an acknowledged pause/end,
and retains ownership through uncertain control or cleanup. Cancellation closes
the managed helper before acknowledging the lease. Detach retains the same
player; reattachment does not start another one. `stop-playback` closes the helper, and
`:plugin-stop media` cleans up the plugin's managed process group. A paused
helper alone does not protect persistent-session retirement.

The host-managed `mpv_backend.py` bridge starts one mpv child in its inherited
process group, communicates through an inherited local socket, and bounds JSON
lines at 64 KiB and its output queue at 256 KiB. Progress is coalesced to at most
two updates per second during playback. Paused settlement creates no polling or
redraw loop. Invalid state, a lost command acknowledgement or truncated output
causes helper cleanup rather than automatic control replay.

The bridge disables user configuration, script loading, automatic sidecars,
playlist-reference loading and the network extractor. Its external window only
displays video: default input bindings, on-screen controls and drag/drop are
disabled so playback mutations pass through the native controller's lease checks.
These settings are not a
decoder or filesystem sandbox; the enabled plugin and installed player run with
the user's ordinary permissions. Codecs and output devices remain mpv concerns.
Pass `--mpv /absolute/path/to/mpv` in the example's `args` to select its executable.
`--headless` selects null audio/video output for deterministic tests and does not
exercise a display or sound device.

Run `python3 docs/plugins/check_mpv_backend.py` with mpv on `PATH` (or `MPV` set
to its executable) for real local-file playback, IPC, bounds and cleanup tests.
`check_media.py` and `check_media_review.py` cover host-call orchestration,
state validation and cancellation without a live service account. CI installs
mpv explicitly; real-backend tests report skips when it is unavailable locally.

## Settings and workspace state

`settings` and `state` are separate explicit capabilities. Configured settings are
immutable for one plugin instance; global preferences belong in user configuration.
The state store holds nonsecret workspace preferences and last destinations. Use
an established credential manager or authentication helper for secrets.

`settings.get {}` returns `{settings:{...}}` for the requesting configured owner.
The optional `settings` mapping on its configuration defaults to `{}` and contains
at most 64 KiB of canonical JSON, eight levels and 4,096 value nodes. Nested JSON
is allowed without a registered schema. Nothing reads a disabled plugin bundle,
scans for settings, or reads state storage during startup.

Registration may include `settings_schema:{fields:[...]}` with at most 64 fields.
Each field has a unique `name`, `type`, and optional boolean `required` (default
false). Names start with an ASCII letter or underscore, continue with letters,
digits, underscores or hyphens, and contain at most 48 bytes. Supported types are:

- `string`, optionally bounded by `max_length` Unicode scalar values and an
  `enum` of one to 64 distinct strings, each at most 4,096 UTF-8 bytes.
- `integer`, a signed 64-bit value with optional inclusive `min` and `max`.
- `boolean`.

A supplied schema rejects unknown configured keys, missing required fields and
values of the wrong type. There are no defaults, coercion, regular expressions,
references or automatic migrations. Schema text is bounded at 64 KiB with the
same eight-level/4,096-node limits. Optional schema/constraint fields may be null
to mean absent; `required` must be a boolean when provided. Validation happens
before any commands or capabilities are installed. Diagnostics do not echo
configured values or unrecognized keys.

State is one whole document per exact configured plugin ID:

```json
{"version":1,"data":{"destination":"sample"}}
```

`version` is the plugin's unsigned 32-bit migration version. `data` is bounded
JSON. `state.get {}` returns `{revision,document}`; an absent document returns
`{"revision":"s:missing","document":null}`. Otherwise the opaque revision is a
hash of canonical document content. The host does not interpret migration
versions, restore live handles, or migrate state automatically.

`state.set {expected_revision,document}` atomically replaces the observed document;
`state.delete {expected_revision}` deletes it. Both return the resulting state
metadata. A different current revision yields `conflict`, preserving the current
file. This compares content: changing away and back to identical canonical JSON
has the same revision. It is not an edit sequence or an exactly-once operation ID.
Read state and make a new explicit decision after a conflict or uncertain result.
Do not retry a mutation merely because a response was lost.

Canonical documents contain at most 1,044,480 bytes (1 MiB minus 4 KiB of wire
headroom), 16 levels, 16,384 value nodes and 1,024 entries per object or array.
Arbitrary maps are permitted inside `data`; the document and request envelopes
remain strict. Structural checks run before materializing unbounded JSON trees.
The committed state quota remains below 1 MiB per owner.

Integer literals must fit the range from signed 64-bit minimum through unsigned
64-bit maximum; larger integer literals are rejected before parsing can round
them. Decimal or exponent numbers use finite IEEE 754 double precision. Store
larger exact identifiers as strings. These semantic checks supplement the schema.

Storage lives under the configured workspace state root, normally
`.runyte/plugins/<configured-id>/state.json`, separate from tracked `context/`. Private directories and files use descriptor-relative access
without following links. A stable nonblocking lock serializes cooperating hosts'
content checks and writes; contention returns `busy`. Atomic replacement and
file/directory synchronization preserve the old document before promotion. A
fixed temporary leaf bounds interrupted-write leftovers and is recovered only
on an explicit state operation. Corrupt or unsafe existing storage is refused,
with static diagnostics, rather than silently overwritten.

Only one state operation per configured owner may run, including unfinished
workers from a stopped generation. Work runs off the editor loop, reserving a
shared local-worker slot and 16 MiB of the existing payload budget until actual
completion. The host deadline is ten seconds. Before mutation admission,
cancellation prevents promotion; after an attempted mutation, timeout or failure
can yield `outcome_unknown`. Stopping a plugin discards its late reply while
retaining accounting and the owner gate until the worker finishes. Pending work
protects ordinary quit/retirement. State operations start no periodic timer.

The SDK exposes `get_settings()`, `get_state()`,
`set_state(expected_revision, version, data)` and `delete_state(expected_revision)`.
State replies get two seconds of delivery headroom beyond the host deadline. A
lost mutation acknowledgement is `outcome_unknown`; the SDK neither retries nor
migrates. This delivery margin does not extend an enclosing command’s ten-second
lifetime; longer workflows should run behind an explicit finite job. Plugin
authors must decide which versions they can read and perform
any migration as an explicit conditional replacement.

The standard-library example `preferences.py` uses a validated configured default
and saves one nonsecret destination:

```yaml
plugins:
  - id: preferences
    enabled: true
    api: runyte-experimental-2
    executable: /usr/bin/python3
    args: [/path/to/runyte/docs/plugins/preferences.py]
    capabilities: [settings, state, views]
    settings:
      default_destination: .
```

`:plugin.preferences.open` inspects the current preference.
`:plugin.preferences.remember "sample directory"` saves it and
`:plugin.preferences.forget` conditionally deletes it; open the view again to
inspect the result. Restarting the editor preserves the document. Opening or remembering refuses unknown stored versions and leaves migration
to an explicit future implementation; forget remains an explicit conditional
deletion. It performs no startup state read and does not automatically
recreate an old view or replay an old command.
