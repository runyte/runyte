# Plugin applications

## Status and intended outcome

Active implementation plan, originally written 2026-09-08 against `c7c18bd`
(`Add experimental external-process plugins and versioned API schema`).
Implementation authorized 2026-09-08. Milestones are being implemented in order;
this record remains active until all acceptance gates have evidence.

The objective is an extension system that can host useful applications: a file
manager, a remote file browser/editor with transfers, and a media controller.
Plugins should own application logic and external connections. Runyte should
own editor state, transactional edits, input conventions, layout, rendering,
resource accounting, and integration with persistent sessions. The editor's
responsiveness and coherent interaction remain release requirements.

The implementation is complete when those application patterns work through
documented public operations, without patches for individual example plugins.
It is not complete merely when a larger collection of methods exists.

## Implementation record — 2026-09-08

Implementation began from `e86fe54`, whose runtime matches the `c7c18bd`
foundation inspected above. This plan remains active: the complete application
patterns and all acceptance gates have not been delivered.

The current slice provides:

- Exact epoch routing with epoch 1 remaining the configuration default; atomic
  epoch 2 command registration and explicit capability grants.
- Independent command deadlines, finite jobs, cancellation acknowledgement,
  generation-scoped handles, reliable job lifecycle events, bounded command/job
  histories, per-producer inbound limits and encoded outbound byte accounting.
- Required positional scalar command arguments, workspace/buffer/view contexts,
  dynamic application key scopes validated in both fast-pane variants, a primary
  Enter action, and a Tab action list drawn from runtime command metadata.
- Retained semantic application projections, stable row selection/viewport
  mapping, revision-checked publication, stale displayed-action rejection,
  guarded foreground presentation, ordinary special-buffer retention and readable
  unavailable content after plugin stop. Native snapshots carry diagnostic
  warning/error scopes through bundled frontend protocol 51.
- Paged buffer metadata, pane metadata, revision-bound scalar reads, retained
  immutable text snapshots, explicit atomic single-buffer edits, and separately
  revision-checked pane selections. Invocations issue captured buffer/pane
  handles so background handlers do not have to rediscover a moving active target.
- A Python authoring client, runnable task-list and background-job examples,
  epoch 2 JSON Schema, shared Rust/Python wire fixtures, Python example smoke
  coverage, and CI schema checks. Epoch 1's guide/schema/example are retained.

The public implementation is documented in
[applications.md](../../../docs/plugins/applications.md). Its method list is
intentionally limited to operations backed by current host adapters. The larger
operation tables below remain the approved target, rather than a claim that
those methods already exist.

Concrete decisions in this slice:

- Plugin request IDs are increasing canonical `p:<positive integer>` values in
  send order. This makes duplicate detection bounded while allowing responses
  to complete out of order. Live resource handles remain opaque.
- Application views specialize the existing virtual-buffer projection with
  `GeneratedViewIdentity::Plugin`; a second pane-content engine is unnecessary.
  Refreshes use a transaction without user undo history. The initial model has
  stable single-line rows and semantic roles; columns, staged models and patches
  remain outstanding.
- A foreground grant belongs to a pending command and expires on further input,
  pane-target changes or attachment changes. Job acceptance does not extend it.
- Projected models and immutable snapshots share 48 MiB per-owner and 160 MiB
  host-wide retained payload allowances, leaving bounded queue/decoding/copy
  headroom inside the overall 64/256 MiB design budgets. Single-message models
  reserve envelope headroom; the proposed 4 MiB staged publication is pending.
- Job progress is recovered through `job.get` in this slice. General source
  subscriptions, coalescing and resynchronization are not advertised yet.

Milestone 3 now has an initial local file manager, documented below; document
recursive mutations remain. Milestone 4's providers, remote
save reconciliation and transport examples remain unimplemented. Milestone 5
still needs managed processes, terminal/external handoffs, activity leases,
settings/state and the plugin manager. Milestone 6 still needs the broader SDK,
non-Python example, full conformance matrix and complete application performance
and supported-platform evidence. Milestones 1–2 also retain their full overload,
real-attachment and performance acceptance work; the implemented primitives do
not by themselves complete those gates.

Validation: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, both schema checkers and canonical workspace coverage passed on
Linux. Ordinary and instrumented suites each passed 3,097 tests with 33 ignored;
canonical line coverage is 105,199 / 114,783 (**91.65%**), above the unchanged
89% floor. See the [coverage register](../../reference/test-coverage.md).
The existing socket/process suite requires native local socket access in this
environment; a sandboxed handshake write returned `EPERM` and was rerun natively.

The real PTY task-view check exposed an inherited palette gap: dynamic commands
appeared as completions but Enter resolved only built-in names. Palette acceptance
now resolves runtime metadata before the shared parser/executor. Keyboard-driven
tests cover both epochs, correctable argument errors and quoted argument delivery.

The final release comparison against `c7c18bd` completed 120 startup samples and
fifteen ten-second idle windows across disabled plugins, quiescent epoch 1/2
examples and the actual visible task list. Every idle window had zero terminal
writes. Hardware, binary identities, medians, idle ranges and measurement limits
are retained in the [performance register](../../reference/startup-performance.md).
These representative measurements do not complete the latency/workload gates.

Native macOS validation and the full application workload matrix are required
before this record can move to `completed/`.

## Second implementation round

The foundation was committed as `0731e51` (`Add epoch 2 plugin application
foundation`). This round adds bounded asynchronous local directory reads,
revision-checked pages and retained entry identities, regular-file plan intents,
native filesystem confirmation/reconciliation, and explicit asynchronous text-file
opens that preserve live unsaved buffers. `docs/plugins/files.py` exercises these
operations as a retained local manager. The public schema and shared wire fixtures
cover the additions.

This round deliberately leaves recursive filesystem mutations, asynchronous
application of confirmed plans, document save/close/create, input prompts/forms
and the remaining milestones active. Reads and preparation run off the editor
loop; application uses the existing interactive `FsPlan` path.

A comprehensive subagent review covered the committed foundation and this round.
All six findings were fixed: static symlink aliases now use consistent canonical
plan identities and final containment checks; application confirmation preserves
active and inactive dirty directory projections; snapshot close cannot clear
another resource's deadline; half-open selections exclude the next unselected
row; Python cancellation has reserved dispatch capacity; and terminal job history
uses completion order independently of handle spelling. A focused re-review found
no further blocking production issue. Rust regressions live in
`src/workspace/host/tests/plugin_applications.rs` and its `plugin_filesystem.rs`
module; the SDK saturation regression lives in `docs/plugins/check_applications.py`.

`cargo fmt --check`, warnings-as-errors Clippy and the ordinary full suite passed:
3,108 tests, with 33 ignored. Both public schema/example checkers passed (eight
epoch 1 checks and four epoch 2 checks). A native 120×40 PTY smoke test used an
isolated temporary workspace and configuration, displayed the local file manager,
confirmed a create through the actual frontend, opened a text file and returned
to the retained view. The settled view emitted no
terminal bytes during its two-second observation window. This functional debug
smoke is not a replacement for the plan's release performance matrix. Canonical
Linux coverage also passed all 3,108 tests: 105,891 / 115,525 lines (**91.66%**),
above the unchanged 89% floor; see the coverage register.

## Native interaction round

The next work item adds bounded prompts, filterable choices, confirmations and
forms through the `interaction` capability. Opening returns a surface handle;
completion creates a fresh short `ui.submit` callback, keeping human waiting
separate from control deadlines. Only accepted callbacks receive foreground
presentation authority. Text/secret/boolean/choice fields use declarative bounds.
Secret text is masked before every snapshot, excluded from macro/history storage,
and redacted from debug input traces before and after dispatch. Native popup
takeover, detach, source closure, dismissal and owner stop cancel input once.

The local file manager now offers native destination prompts as contextual
actions, then uses the existing filesystem confirmation. Schema fixtures, SDK
callback routing, public-message example smoke tests and host regressions cover
the path. Review additionally corrected native picker semantics and selection
preservation, cancelled-callback authority and shared surface payload accounting.
Final subagent re-review found no blocking issues. Formatting, Clippy, the full
suite (3,116 passed, 33 ignored), both schema checkers and canonical Linux coverage
(106,407 / 116,087 lines, 91.66%) passed. A native PTY smoke exercised destination
entry, prepared confirmation, text-file opening and return to the retained view;
a two-second settled observation produced no terminal output. Revision-tagged asynchronous
field validation will build on the observation/subscription boundary; it is not
advertised as delivered here.

## Explicit local document round

`buffer.create/save/close` now complete the first local document adapter. Named
new documents retain no accepted saved baseline until their first write, including
empty text. Saves validate an explicit revision and known disk conflicts before
the normal undoable whitespace hook, then capture immutable text and disk identity.
A bounded blocking worker performs the conditional atomic write. Completion adopts
only the captured baseline, preserving newer edits and undo. Host-owned jobs
retain pending protection and payload charges through cancellation, deadline
expiry and owner stop; uncertain writes stay dirty until explicit reconciliation.

Review corrected quit/reload/discard/`--wait` refresh seams, file-observation
baseline races, host-owned cancellation acknowledgement, whitespace hook ordering,
empty-document baselines and preservation of actionable save warnings. The public
schema/fixtures and `documents.py` exercise explicit create/save/close. Automated
regressions cover the lifecycle in both the App and workspace-host boundaries.
Formatting, warnings-as-errors Clippy, both schema/example checkers and the full
suite passed (3,132 tests, 33 ignored). Canonical Linux workspace coverage is
106,891 / 116,639 lines (91.64%), above the unchanged 89% floor. Final subagent
re-review found no further concrete defects in this local lifecycle round.
Recursive mutations, asynchronous filesystem apply and all later milestones
remain active.

## Asynchronous filesystem application round

Confirmed application filesystem plans now start host-owned jobs with a reliable
`filesystem.started` event. Disk application, moved-file baseline inspection and
bounded explorer refreshes run in one worker; completion applies prepared facts
against captured identities and revisions. Dirty/newer text is preserved. Git
reconciliation enqueues worker-validated paths without editor-thread path IO.
Capacity failure preserves the confirmation. Accepted work retains its protection
and accounting through detach and owner stop; recovery details survive completion.

Cancellation uses an atomic queued/running transition. It is refused once the
worker starts a filesystem mutation; deadlines cannot interrupt an OS mutation.
This keeps completion truthful without claiming rollback. The pending-operation
barrier covers filesystem writes, new opens/publication, filesystem-buffer close,
reload/discard, `--wait` completion, idle retirement and quit. Ordinary editing,
scratch/special-buffer retirement and detach remain available.

Review corrected special-buffer eviction loops, late-open reconciliation races,
scratch save-as admission, cancellation ordering, outbound job-start failure and
residual Git path IO. Recursive operations and the later application milestones
remain active. Final subagent re-review found no further concrete defects.
Formatting, warnings-as-errors Clippy, both schema/example checkers, the full suite
(3,142 passed, 33 ignored) and canonical Linux coverage (107,502 / 117,376 lines,
91.59%) passed. A native PTY smoke confirmed a create, opened text and returned to
the retained manager; a two-second settled observation emitted no terminal bytes.

## Investigation: existing foundation and missing boundaries

The current [guide](../../../docs/plugins.md) and
[epoch 1 schema](../../../docs/plugins/runyte-experimental-1.schema.json) define
the shipped contract. The [completed minimal-plugin plan](../completed/PLAN_MINIMAL_PLUGINS.md)
explains its original scope; current source takes precedence.

| Area | Exists at the inspected commit | Required extension |
| --- | --- | --- |
| Runtime | `src/plugin.rs`: external child, bounded newline JSON, asynchronous IO, exact version handshake | Multiplexed requests, independently managed jobs, cancellation, negotiated capabilities and fair delivery |
| Ownership | `WorkspaceHost` owns workers; startup occurs once per host, not per attached TUI | Host-owned views, providers, jobs and lifecycle cleanup, including protected remote work |
| Commands | `CommandId::Plugin`, `App::execute`, live palette metadata and validated keymaps | Commands independent of editable buffers, typed arguments, application-local actions and contexts |
| Editing | Captured invoking text/selections; `apply_expected_transaction` applies explicit-target edits | General bounded reads and explicit revision-checked edits, with separate pane-selection targeting |
| Observations | Issued-buffer revision/closure watches, ordered bounded delivery | Buffer, pane, view, job and attachment subscriptions with baseline/resynchronization rules |
| UI | Generated buffers, pane-backed lists, picker/confirmation/input overlays, semantic snapshots | Plugin-owned instances of those surfaces, stable row/action IDs and versioned updates |
| Files | Local file loading/saving, directory buffers and confirmed filesystem plans | Public local-file intents and provider-backed remote documents with reliable asynchronous saving |
| Processes | Plugin children and integrated terminal sessions already run asynchronously | Bounded managed helper processes and lifecycle links for application backends |
| Documentation | Epoch 1 JSON Schema, prose contract, Python example and schema checker | Epoch 2 schemas, authoring SDK, protocol conformance tests and application examples |

Specific constraints found in source:

- `src/workspace/host.rs::execute_expected_command` checks a prepared frame,
  active buffer and revision. It must remain an interactive-client boundary;
  exposing it would recreate the wrong targeting model for background work.
- `apply_expected_transaction` resolves a buffer independently of focus and
  calls `App::apply_to_buffer`. That path maps selections in all panes and
  updates syntax/language-service state. Its adapter must validate raw ranges
  before `Transaction::new`, which otherwise drops overlaps. Undo grouping
  remains an explicit responsibility of extension edits.
- `src/app.rs::Pane` holds a buffer and optional terminal session, with separate
  selection and viewport state. A terminal is not a buffer. The proposed native
  application UI can use special buffers; adding a generic third pane-content
  engine is not a prerequisite.
- `src/picker.rs::ListPurpose` distinguishes picker, choice, manager and report.
  `PickerItem.index` is an internal producer index, not an extension identity.
  Public rows need stable opaque IDs mapped to those internal values.
- `src/snapshot.rs` already separates presentation from live state. Plugins must
  provide semantic models; only host adapters produce snapshots. Ratatui types,
  prepared geometry, core Rust enums and `src/protocol/` DTOs remain private.
- The UI vocabulary requires persistent navigable content to retain ordinary
  buffer movement, selections, search, copying, splits, help and management.
  A transient picker is not a substitute for an application's main view.
- Current worker deadlines allow one ten-second invocation. Keeping a process
  alive already supports state, but that invocation model cannot describe an
  upload, interactive form or playback lifetime.
- Plugin IO runs off the editor loop, but copying snapshots and accepting
  edits/models can still consume editor-thread time. Process isolation alone
  does not guarantee responsiveness.
- The terminal compatibility register explicitly excludes Kitty graphics,
  sixel and iTerm images. Sending raw escape sequences through a plugin view
  cannot create an embedded video feature.

Read alongside this plan: [UI vocabulary](../../reference/ui-vocabulary.md),
[keymap register](../../reference/helix-keymap-v1.md),
[terminal compatibility](../../reference/terminal-compatibility-v1.md),
[startup/idle measurements](../../reference/startup-performance.md),
[coverage invariant](../../reference/test-coverage.md), and the user guide's
workspace, directory-buffer, terminal, UI and plugin sections.

## Product decisions

### One runtime, semantic native UI

Keep external processes and UTF-8 newline-delimited JSON. Any language can
implement the contract, and existing Tokio/Serde infrastructure remains useful.
Do not add Lua, WASM, a browser engine, or a second plugin transport in this
program of work. Audio/video bytes never travel through the editor JSON channel.

Offer two ways to present applications:

1. Native special buffers with structured rows, text, semantic styling, actions,
   forms and transient overlays. This is the primary API and gives applications
   Runyte's navigation and visual conventions.
2. Explicitly requested integrated terminal sessions for existing TUI programs.
   Runyte retains its terminal input and lifecycle rules; a plugin cannot draw
   directly on the outer terminal or read arbitrary input from another terminal.

These cover substantial applications without requiring an arbitrary widget tree
or a replacement layout engine. Native audio is delegated to a helper/player;
video initially uses an external player window or browser. A native Runyte
controller for that playback is in scope. Inline video pixels are a separate
renderer project, not an implied property of this API.

### Application ownership and persistence

One enabled plugin instance belongs to one workspace host. Different workspaces
can have different instances; attachment changes never create new instances.
There is no new global plugin daemon. A plugin that controls an existing music
player connects to that external service rather than starting playback per TUI.

The host retains application models, jobs and provider-backed buffers through
detach/reattach. It retains pane-specific focus, selections and viewport state
separately. A view may be shared by panes. Closing a pane releases that pane's
attachment to a view; it does not automatically close the view or stop its job.

Clean generated views follow the existing bounded special-buffer retention
policy. Visible views are not evicted. Eviction emits a lifecycle event and
invalidates the handle; the plugin may recreate the view on a later command.
Application registration alone does not pin an idle persistent session. Dirty
documents and active finite jobs count as protected state. Deliberate playback
or another continuing service may acquire a bounded, renewable activity lease,
shown in session health and respected by normal shutdown/idle policy. Forced
shutdown remains available and explicitly cancels those resources.

Host restart loses live IDs and processes. A small nonsecret plugin state store
supports deliberate restoration of preferences and last destinations, not
transparent restoration of in-flight requests or an exactly-once guarantee for
remote operations. Do not promise editor crash recovery as part of this plan.

## Epoch 2 protocol and authoring contract

Introduce `runyte-experimental-2`, retaining epoch 1 behind its existing adapter
for the transition. Existing configuration defaults to epoch 1; an explicit
`api` field selects epoch 2. Each connection uses one exact epoch. Retain the
epoch 1 schema and example and test that they still run. No automatic downgrade
that silently removes requested capabilities.

Epoch 2 keeps a hello/register/registered handshake with application metadata,
declared capabilities and effective limits. Register commands, provider kinds
and permitted view kinds atomically. Return the granted capability set; fail
registration if a required capability is unavailable. Optional capabilities
allow a plugin to omit a feature intentionally. Unknown request methods return
`unsupported`; malformed envelopes or repeated protocol abuse terminate the
connection. Unknown host fields remain ignorable; plugin request fields remain
strict. Compatible additions need schema updates; incompatible shape or semantic
changes require another epoch. No stable Rust ABI is introduced.

Use a common envelope in both directions:

```json
{"type":"request","id":"p:17","method":"buffer.read","params":{"buffer":"b:4","expected_revision":"r:9","from":0,"to":120}}
{"type":"response","id":"p:17","result":{"buffer":"b:4","revision":"r:9","from":0,"to":120,"text":"..."}}
{"type":"response","id":"p:18","error":{"code":"stale","message":"Buffer changed","data":{"actual_revision":"r:10"}}}
{"type":"event","subscription":"s:2","sequence":"e:8","event":"buffer.changed","data":{"buffer":"b:4","revision":"r:10"}}
```

These are proposed envelope examples, not valid epoch 1 messages. Final schemas
must define each method's params/result and each event payload; untyped arbitrary
maps are reserved for explicitly plugin-owned settings/state. An ID is unique
within its sender's connection generation; host and plugin ID spaces are
separate. Responses contain exactly one of `result` and `error`, never both.
Requests may complete out of order and receive at most one terminal response.
Disconnect leaves outstanding requests failed/unknown locally; no final response
is promised to a dead peer and no mutation is automatically replayed.

Long work returns a job handle promptly. The request's successful response means
the job was accepted, not completed. `job.changed` carries progress and exactly
one terminal state on a surviving connection. `job.get` recovers the current
state if observations were coalesced. Command invocations carry a captured context
and may return an immediate result or an accepted job. Preserve the originating
action echo without overwriting feedback from newer user input.

Separate errors into `invalid_argument`, `unsupported`, `capability_denied`,
`not_found`, `closed`, `stale`, `conflict`, `read_only`, `busy`, `limit_exceeded`,
`cancelled`, `timeout`, `unavailable` and `internal`. Provider operations may
also return `outcome_unknown` when an external write may have happened. A user
can distinguish malformed input, refused work, and a failed external operation.
Messages shown to users must not contain tokens, passwords or raw protocol dumps.

### Identity, context and concurrency

All buffer, pane, view, row, subscription, job, provider-document and process
handles are opaque strings scoped to the issuing connection generation. Keep
ownership in the host and reject a handle issued to another instance. Closing
or evicting a resource invalidates it; restarting never reuses an old handle.
Provider resource keys may be stable across connections but are separate from
live handles. Do not expose host slot numbers as a promised numeric format.

Offsets remain Unicode scalar positions. Edit ranges are half-open; selection
anchor/head retain direction and invoking contexts include operative spans.
Distinguish text revision, pane-selection revision, view-model revision and
query revision. Focus or geometry changes do not invalidate a background edit.
Changing a pane's selections requires that pane's ID, displayed buffer and
selection revision; plugin code must never change whichever pane is now active.

Command contexts are declared as `workspace`, `buffer` or `view`. A workspace
command can run with a terminal or read-only buffer active. Capture pane/view
identity and attachment generation for any foreground UI request. Methods that
need visible user interaction fail with `no_frontend` or `context_changed`
(additional defined error codes) after detach or focus changes; they do not wait
indefinitely and steal focus on the next attachment. Jobs remain independent.
An explicit foreground command can present an existing view after reattachment.

No synchronous callback from editor mutation, rendering, key dispatch or saving
may wait for plugin output. Host methods create asynchronous continuations where
necessary, including provider saves, and release the event loop immediately.

## Public operations to implement

Names below are proposed epoch 2 methods, not promises about Rust module names.
Each group has a capability and standalone/persistent behavior tests.

| Group | Operations | Contract |
| --- | --- | --- |
| Workspace | `workspace.info`, `buffer.list`, `pane.list` | Bounded metadata pages; explicit handles; no unrestricted filesystem scan |
| Text | `buffer.read`, `buffer.snapshot.open/read/close`, `buffer.edit` | Revision-bound reads and explicit-target single-buffer transactions |
| Documents | `buffer.open/create/save/close` | Explicit target/destination; existing save/dirty-close rules; asynchronous file IO |
| Selections | `selection.get/set` | Explicit pane, buffer and selection revision; no focus requirement |
| Presentation | `pane.show`, `view.create/get/publish/patch/close` | Host-owned layout; foreground presentation separate from background model updates |
| Interaction | `ui.prompt`, `ui.pick`, `ui.form`, `ui.confirm`, `ui.dismiss` | Typed bounded surfaces, structured accept/cancel result, captured owner context |
| Feedback | `notification.publish`, `job.get/cancel` | Retained errors, bounded progress, owner-labelled cancellation |
| Job ownership | `job.create/update/finish` | Host-issued handles, validated state transitions and a terminal result for accepted work |
| Observations | `event.subscribe/unsubscribe` | Baselines, connection ordering, explicit delivery class and resynchronization |
| Local files | `filesystem.list/stat`, `filesystem.prepare/apply/cancel` | Asynchronous reads; typed mutation intents routed through existing prepared plans |
| Remote documents | Provider registration plus host-to-plugin `resource.stat/read/write` | Plugin transports bytes; host owns editable text, saved baseline and dirty state |
| Helpers | `process.start/write/close`, `terminal.open`, `external.open` | Explicit executable/arguments or validated URL; host-managed lifecycle and bounded IO |
| State | `settings.get`, `state.get/set/delete` | Namespaced validated configuration and bounded nonsecret JSON state |

Do not expose arbitrary `App::execute`, keystroke replay, frontend transport
messages, raw Ratatui widgets or generic interception of every built-in command.
Add semantic operations when an application needs them. Built-in file and pane
operations remain available through their existing UI regardless of plugin state.

### Reads, edits and selection changes

Small reads return a revision and exact range. Large reads use a bounded retained
rope snapshot and explicit chunk cursor, with one immutable revision for the
entire read. Never concatenate chunks from changing revisions. Snapshot expiry
returns `closed`; releasing/closing/disconnecting frees retained memory. Unicode
boundaries must hold at every chunk boundary, including escaped JSON text.

`buffer.edit` includes buffer, expected text revision and a vector of explicit
changes. Check every range, ordering/overlap, text limit, liveness and editability
before constructing a transaction. Apply all changes or none, commit any earlier
insert undo group, and make one undo step. Return resulting revision and applied
change summary. Empty transactions succeed as no-ops. Do not offer a long-lived
open undo group across network round trips. Multi-buffer atomic edits are outside
this first application API; independent edits must expose independent outcomes.

Runyte continues to map current selections in every pane after mutation.
`selection.set` is a separate optional operation with explicit pane preconditions.
An edit succeeding while a subsequent selection change is stale remains a
successful edit; never imply that the pair was atomic.

### Native views and actions

Add a generic plugin-owned special-buffer kind and a host model keyed by view
ID. A view declares `document`, `list` or `dashboard` purpose and consists of
bounded semantic text/rows, column labels, optional detail/preview, status and
registered actions. Theme roles use a documented allowlist such as ordinary,
muted, heading, warning and error. No ANSI, terminal controls, arbitrary RGB,
HTML, script expressions or plugin-selected screen coordinates in native views.

The host materializes text and semantic spans, retaining row/action identities
separately from rendered labels. Reuse list matching and generated-page behavior;
do not serialize `PickerItem`, `BufferKind` or `SnapshotRow`. All projected text
changes still go through transactions, but generated refreshes do not mark the
view dirty or add user undo entries. Editable application data is a separate
scratch or provider-backed document, not an editable decoration model.

`view.publish` replaces a complete model at an expected model revision;
`view.patch` atomically inserts, updates, removes or reorders stable row IDs.
Validate a complete update before publication. Large models may be assembled in
bounded staging chunks and committed once. Stage expiry/cancellation leaves the
previous view intact. Preserve per-pane selection and viewport by stable row ID;
if a row disappears, choose the nearest surviving row deterministically. A hidden
view receives model changes without generating frames or requesting focus.

Actions carry stable command/action IDs, selected row IDs and observed model
revision. Accepting a displayed row must never act on a different row after a
refresh. Filtering/sorting and pointer hit-testing resolve against the model
generation the user saw; an obsolete action is rejected or requires refresh.
For remote search, send query revisions and accept only matching results. Keep
previous results visible but non-actionable while a new query is outstanding,
following the existing Finder contract.

Native views retain Normal/Select behavior and normal buffer management.
`Enter` performs a declared primary action where the scope permits it; `Tab`
opens the existing contextual action menu. Persistent views do not permanently
capture printable keys to filter: use an explicit filter action. Preserve host
window/application prefixes, cancellation, counts and macro ownership. Custom
bindings use dynamic scopes validated by the existing effective-scope validator
in both fast-pane variants. Never introduce a second dispatch/help/hint table.

Typed scalar prompts and forms reuse the interaction line/input overlays. Forms
support text, masked secret, boolean and bounded choices, required fields and
simple declarative validation. Plugin-side remote validation runs asynchronously
and is revision-tagged. Secret values are excluded from history, snapshots sent
to noninteractive clients, state persistence and diagnostic logs. Only one input
surface can own input; another request returns `busy`. Dismissal, owner closure,
detach and plugin failure settle the request once as cancelled.

### Command registration and loading

Keep `plugin.<configured-id>.<local-name>` names and reserve host lifecycle names.
Epoch 2 registration adds context, argument schema, description and view actions.
Support a bounded argument subset first: strings, booleans, integers and enums;
use Runyte's colon quoting/parser rules, never shell evaluation. Forms collect
richer input. Registration and all configured bindings succeed atomically or
leave no callable remnants. Removal invalidates IDs and updates palette/help/hints
together. Re-registration requires explicit restart, not an in-place mutation.

Continue explicit configuration of executable, args, epoch, capabilities and
plugin-specific settings. Validate settings against an optional registered schema.
Provide a local bundle convention containing the executable/script, README,
settings example and protocol requirement. Installation remains copying a bundle
and enabling its path; a manifest/package resolver is not needed for this API.
Read no disabled plugin bundles and run no discovery scans. Start enabled workers
after initial standalone presentation, once in persistent host startup.

Add a host-owned plugin manager showing configured/running/failed state, granted
capabilities, jobs and bounded diagnostic summaries, with explicit stop/restart.
Stopping unregisters commands, removes watches and input surfaces, cancels jobs,
and makes retained plugin views visibly unavailable. Preserve their readable
content until ordinary close/eviction. Dirty provider documents remain editable
and recoverable, with save unavailable until their provider is explicitly rebound.
Restart creates a new generation and never replays old actions. Rebinding a dirty
document requires the same provider/resource identity and fresh remote validation.

### File manager and remote documents

A local file-manager plugin uses paged directory metadata and stable entry IDs to
publish a list. Opening an entry calls the host's ordinary document/directory
operation. Create/copy/move/trash/delete intents go through an adapter over
`FsPlan`: prepare, inspect a host-rendered confirmation, revalidate and apply.
The plugin cannot synthesize the user's confirmation response. Failed/partial
external operations retain actionable recovery details. Never promise undo for
filesystem side effects merely because an editor buffer has undo.

This reuses the existing filesystem data-safety boundary. It does not resolve or
move [the deferred hostile-process symlink-race issue](../../issues/deferred/fs_plan_symlink_race.md).
Capability declarations are not a new OS confinement guarantee. Preserve the
existing supported-platform behavior and its known limitation.

Remote transports belong to plugins. Add provider-backed documents identified by
`(configured plugin identity, provider name, canonical resource key)`, separate
from local paths. A remote path must not leak into local Git discovery, overwrite
a local file, or masquerade as the URI of a local LSP document. Start with syntax
highlighting and ordinary editing; remote LSP needs a separately declared provider
mapping and is not inferred. Define display labels without embedded credentials.

Opening obtains metadata, an opaque remote version and bounded text chunks before
publishing a document. Reopening the same resource reuses its live buffer.
Text resources use an explicitly declared encoding, initially UTF-8, with newline
handling documented; unsupported/binary resources are downloads/external opens.
Transfers of arbitrary binary data remain in the plugin/helper process. Use a
host-issued private staging destination for a download and publish to a user
destination only through the existing confirmed filesystem workflow.

Saving captures an immutable local revision and previous remote version and
starts `resource.write`. The provider returns a new remote version and a commit
outcome. Runyte marks only the captured text as the saved baseline. If the user
edits during upload, the newer local text remains dirty. Save, save-and-close and
`--wait` completion must await confirmed success; they cannot close/acknowledge
on job acceptance. Save failure or cancellation preserves text and dirty state.
Only one save per resource is active; another save returns `busy` or an explicit
queued-save job, never a hidden duplicate upload.

Providers declare `conditional_write` and `atomic_replace` support independently.
Use remote preconditions where genuinely supported. An FTP/SFTP preflight stat
followed by upload is not an atomic compare-and-swap. Where safe conditional
writes are unavailable, default to refusing unattended overwrite; offer explicit
foreground confirmation or save-as, describing the remaining race. Do not claim
that temporary upload plus rename alone detects external edits. A disconnect
after sending a write can leave its outcome unknown; retain dirty data, re-read
remote state and reconcile before another write. Never retry a mutation solely
because its response was lost.

The reference remote application should implement one transport first: SFTP,
using an established plugin-side library or helper with host-key verification
and existing credential mechanisms. The public provider API must not encode SSH
assumptions. Add FTP/FTPS through the same interface as a subsequent adapter;
label transport and overwrite guarantees accurately. Plain FTP is an explicit
connection choice, not an automatic fallback. Transfer progress, cancellation,
conflicts and remote mkdir/rename/delete use the same job and confirmation model.
No SSH/FTP library becomes a Runyte core dependency.

### Jobs, events and scheduling

Separate short control requests, finite jobs and continuing services. Control
requests default to a ten-second deadline. Finite jobs have explicit negotiated
deadlines up to one hour and can report bounded progress. Continuing services
use a renewable ten-minute activity lease instead of an infinite pending request;
lease expiry cancels their protected status and invokes the defined cleanup path.
Do not create heartbeat/poll timers for disabled or quiescent plugins.

`job.create` reserves a host-owned handle, budgets and originating command context
before work starts. A command response may return that handle; it cannot invent
one. Host-initiated provider calls receive a handle already reserved by the host.
Job updates/finish are accepted only from their owner and for a valid state
transition. Owners receive reliable terminal job events without needing to race
a separate subscription request; progress subscriptions may coalesce. Foreground
context grants do not become permanent merely because a job outlives its command.

`job.cancel` records cancellation once, requests cooperative stop and waits up to
two seconds for acknowledgement. On timeout stop the unresponsive owner and
settle its remaining work. A result that committed before cancellation may return
success; after cancellation is accepted, no new editor mutation is admitted for
that job. Remote side effects may already have happened and must report
`outcome_unknown` when appropriate. Stop/restart/shutdown use the same machinery.

Extend subscriptions with explicit source filters and a baseline captured in the
same host turn as subscription creation. Use a monotonically ordered connection
sequence, plus source revisions. Topics include buffer open/change/save/close,
pane target/selection changes, view close/eviction/action, attachment changes,
job state and helper exit. Do not publish every keystroke or rendered frame.

Use two delivery classes:

- Reliable control: responses, lifecycle transitions and accepted user actions.
  Bounded reserved queue capacity; if delivery is impossible, disconnect the
  owner rather than silently dropping a terminal result or action.
- Coalescible state: buffer invalidation, view query, viewport and progress.
  Retain the latest state per source. If a bounded source table overflows, send
  a reliable `resync_required` marker and suspend that subscription until a
  requested baseline reset. Sequence gaps/coalesced counts are explicit; no
  claim of an edit log or replay. If even the marker cannot be delivered, stop.

Queue a mutation response before its resulting invalidation. Unsubscribe is
idempotent: earlier queued observations precede its acknowledgement; none follow
for the old subscription. Re-subscribe obtains a new identity and baseline.
Closing a resource emits a final reliable lifecycle transition and invalidates
dependent requests. Cross-plugin ordering is not guaranteed.

Use per-plugin queues and fair scheduling so one producer cannot monopolize the
shared event channel. Decode/validate large messages and build projections off
the editor loop. The host publishes bounded prepared results and yields between
batches. Do not copy every full view into every attached frame: retain models and
render visible rows, using existing damage/dirty scheduling. Frame production
stops while detached; transfer and document state may still advance.

### Helpers, authentication, media and state

`process.start` launches an argument vector without a shell by default and
returns a managed helper handle. Bound output chunks, stdin writes and retained
logs. The helper's stdout is separate from the plugin protocol. Add process-group
cleanup on Linux/macOS, reusing applicable terminal/Git lifecycle experience and
tests; do not promise control of descendants that deliberately escape the group.
A plugin can spawn its own children under normal OS permissions, but lifecycle
guarantees apply only to host-managed helpers. Terminal sessions use their existing
PTY owner and explicit retention policy; ordinary plugin helpers stop with owner.

Authentication and service-specific network calls remain plugin-side. Support
masked prompts and an explicit `external.open` operation for browser login or a
player, limited to declared URL schemes/handlers and foreground context. Default
to the platform credential manager or a plugin's established authentication
helper. Do not add a plaintext token store to Runyte. Capability checks restrict
host API calls, not the network/filesystem access of the already trusted process.

Provide an optional native media-controller example backed by an external player.
It owns a playlist/list view, current-item dashboard, play/pause/seek actions,
progress and a cancellable playback lease. A local media file is the deterministic
integration target. mpv's documented [JSON IPC](https://mpv.io/manual/stable/#json-ipc)
is a candidate helper interface; codec and output behavior remain in the helper.

Spotify is a service adapter, not a new editor capability: browse metadata and
control an authorized playback device. Its [start/resume API](https://developer.spotify.com/documentation/web-api/reference/start-a-users-playback)
requires Premium and an available playback device. Verify current authorization,
scope, application access and account requirements when implementing the adapter;
do not claim that adding editor methods supplies an audio decoder or service access.

YouTube browsing/control uses the same native views and external-open/helper
boundary. The official [IFrame API](https://developers.google.com/youtube/iframe_api_reference)
is a web player, not a terminal widget. The baseline is opening playback in an
external browser/player; in-editor playback controls require a backend that
actually exposes them. Browser handoff alone must not be labelled a full in-editor
player. A future inline-video milestone would need a separately reviewed graphics
surface, decoder/backend, terminal capability negotiation, resize/clipping,
bandwidth budgets and non-graphics fallback. It is outside this plan's completion
claim. External-service references were checked 2026-09-08 and must be rechecked.

Keep plugin logs bounded and opt-in for raw stderr, labelled by owner, with no
request bodies or credentials logged by the host. Normal failures publish concise
diagnostic summaries. Nonsecret state belongs under private ignored workspace
runtime storage, namespaced by plugin ID, with quotas and atomic replacement;
global preferences belong in user configuration. Tests inject temporary roots.
State migrations are plugin-versioned and cannot resurrect connection handles.

## Initial resource budgets

These are proposed starting limits, to be encoded in schemas/config validation,
advertised in the handshake and tuned only with recorded measurements. Epoch 1
keeps its current limits. Limit errors are structured and apply no partial edit
or model update. Count queued bytes as well as message count.

| Resource | Epoch 2 starting limit |
| --- | ---: |
| Enabled instances | 8 per workspace host |
| Commands / live native views | 64 / 16 per plugin |
| Outstanding control requests / finite jobs | 16 / 4 per plugin |
| Continuing activity leases | 2 per plugin, 10 minutes each |
| One encoded JSON line including newline | 1 MiB |
| Text or helper-data chunk | 256 KiB decoded; must also fit encoded line |
| One explicit text transaction | 1,024 changes, 512 KiB replacement text |
| Retained text snapshots | 2 per plugin, 16 MiB each, 30-second idle expiry |
| View model / stable rows | 4 MiB / 10,000 per view; larger datasets page |
| Total extension-retained payload memory | 64 MiB per plugin, 256 MiB per host |
| Live subscriptions / watched sources | 32 / 256 per plugin |
| Control queue | 32 messages and 4 MiB per plugin, with reserved terminal-result slots |
| Coalesced-state table | 256 sources and 4 MiB per plugin |
| Pending input surfaces | 1 per attached frontend; no hidden unbounded queue |
| Helpers / retained helper output | 4 / 1 MiB per helper |
| Nonsecret state | 1 MiB per plugin per workspace |
| Active-view publication / visible progress | At most 10 / 2 updates per second |

No per-view timer when nothing is pending. Honor stricter existing picker pacing
rules. Aggregate quotas include staging, retained snapshots, queues and helper
payloads; reject reservations before allocating. These are bounded payload
budgets, not an exact allocator RSS cap. Count limits bound metadata overhead.
On plugin stop/eviction/cancellation, release charges in the same ownership path.
Byte limits do not cap an external program's OS CPU or memory; security sandboxing
and OS quotas remain distinct future work.

## Implementation milestones and acceptance gates

Each milestone must ship its schema, prose, examples and behavior tests with the
implementation. Keep each runnable during development; do not build all method
families before demonstrating a view. Retain epoch 1 regression coverage.

### 1. Protocol, ownership and job foundation

Split wire definitions, IO worker and host orchestration into focused modules
under the existing plugin ownership boundary. Add epoch routing, typed envelopes,
capability checks, handles/generations, independent request/job tracking, queue
quotas and explicit cancellation. Extend the schema checker and add Rust wire
conformance fixtures so schema examples are also deserialized/serialized by the
implementation. Run schema checks in CI with development-only tooling.

Gate: two out-of-order requests, a job longer than ten seconds, cancellation,
duplicate/late results, overload and a crashing process all behave deterministically
while a second plugin and normal editor input remain responsive. Epoch 1's example
still runs unchanged. Detached work retains one owner and one process.

### 2. One native application view and command context

Add workspace-context commands, typed arguments, a plugin-owned list special
buffer, stable rows, atomic model publication, view actions and foreground
presentation. Integrate registry scopes, help, hints, pane history, Navigator,
Finder live resources, split sharing and persistent session previews. Add only
private DTO changes needed by bundled frontends and bump their private protocol
version when required; keep those DTOs out of the extension schema.

Gate: a sample task list opens over either a terminal or document, remains open,
filters via an explicit action, updates without cursor jumps, acts on stable rows,
and survives detach/reattach. Slow refreshes cannot activate obsolete rows or take
focus from another pane. Normal buffer navigation, copy/search and help still work.

### 3. Useful local file-manager application

Add paged filesystem reads, explicit document opens, text reads/edits and
pane-selection operations. Add prompts, choices, confirmations and forms. Connect
filesystem mutation intents to the existing prepared-plan workflow and implement
a runnable local manager with browse/open/create/rename/copy/trash actions.

Gate: the application manages temporary test directories through the public API,
opens files in an explicit destination, and cannot bypass a prepared confirmation.
Cancellation changes nothing; collisions preserve files and report recovery.
Multiple Unicode selections edit atomically in a hidden buffer and undo once.
Large reads never mix revisions. Another plugin's handles are rejected.

### 4. Provider-backed documents and transfers

Add resource identity and asynchronous read/save adapters, remote-version handling,
dirty-baseline reconciliation and provider lifecycle. Implement a deterministic
local test provider and a runnable SFTP example. Add FTP/FTPS as a separate plugin
adapter using the same public methods after the first provider proves the contract.
Add download staging, transfer progress/cancellation and remote mutation confirmation.

Gate: browse, download/open, edit and save a remote text document; edits during an
upload stay dirty; remote conflicts do not overwrite; plugin failure never destroys
local unsaved text. Save-and-close and `--wait` await success. Disconnect after a
remote commit yields an honest unknown outcome. Upload completion while detached
does not require a frame. Automated cases use isolated fixtures, not live accounts.

### 5. Application lifecycle and media backend

Add dashboards/row patches, managed helper processes, existing-terminal launch,
explicit URL/player handoff, activity leases, plugin manager, bounded diagnostic
state and settings/state APIs. Build the local-file media controller as the
reference continuing application. Supply Spotify/YouTube adapter examples or
guides that explicitly separate available controls, service prerequisites and
external playback; do not make live service access a core release gate.

Gate: start/control/stop a managed backend; input remains responsive during output
floods, paused idle causes no redraw loop, detach does not duplicate playback, and
stop/shutdown cleans up owned helpers. A host with active playback reports protected
state; an enabled but quiescent plugin alone does not prevent idle retirement.
Provider documents remain recoverable after explicit plugin restart.

### 6. Authoring kit, conformance and release readiness

Provide a small Python SDK around the public wire contract: a continuous reader,
request correlation, typed errors, cancellation, subscriptions and handler dispatch
that does not block the reader. Include a non-Python protocol example to demonstrate
that the SDK is optional. Avoid generated clients until schemas and actual usage
agree; generated reference documentation is useful immediately.

Publish local installation/enablement instructions, API method/event index,
capability and version policies, lifecycle diagrams, resource budgets, migration
from epoch 1, and complete runnable examples. Make every supported method visible
in the schema and test representative success and failure responses. Document
which behaviors need host state beyond structural JSON validation.

Gate: a plugin author can install and run the local manager and one background
application from a clean clone without editing Runyte, reading Rust internals,
or assuming a live TUI. The provider example has a repeatable local-server setup.
Current docs separate delivered functionality from service adapters and inline
video work that remains outside the API.

## Validation and performance evidence

Use behavior-boundary tests in `tests/` or source `tests` subdirectories, temporary
storage, injected config/cache/state roots, and the checked-in
`src/fixtures/stand-in` with adjacent behavior data. Never execute files a test
wrote. No automated test accesses personal credentials, a real FTP/SSH account,
Spotify or YouTube. Optional manual service smoke tests must document prerequisites
and results separately from deterministic conformance tests.

Required coverage includes:

- Unicode scalar/chunk boundaries, backward/multiple selections, single-step undo,
  stale reads/edits, closed/read-only buffers, changed active pane and selection.
- Command names/arguments/collisions, dynamic scope help/hints, failure cleanup,
  restarted generations, foreign handles and unsupported capability/method errors.
- Model/query revisions, stable row actions under refresh/filter/reorder, split
  views, paging, eviction, selection/viewport preservation and malformed patches.
- Prompt/form cancel, secret redaction, busy UI, detach during input, stale context,
  private snapshot encoding/decoding, narrow terminal layouts and theme changes.
- Subscription baseline/order/coalescing/resync/unsubscribe, close events,
  queue fairness, cancellation/result races and a flood beside a quiet plugin.
- Local filesystem failure/recovery, remote conflicts, upload during local edits,
  unknown commit outcome, provider restart/rebind, dirty close and `--wait`.
- Process timeout/exit/flood, managed-helper group cleanup, lease expiry, pending
  job protected state, detached completion and repeated real TUI attachments.
- Empty/disabled plugin configuration creates no processes, queues or timer wakeups.

Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`,
the schema/conformance checks, and `cargo llvm-cov --locked --workspace`. Keep the
canonical total Lines result at or above the enforced 89% floor on Linux and
macOS; never lower the floor or count documentation checks as Rust coverage.
Record native platform limitations honestly and require supported-platform CI
before final completion. Existing terminal, local-protocol and persistent-host
regressions remain mandatory as their ownership paths change.

Compare release builds against `c7c18bd` using `benchmarks/plugins.py` and the
existing startup/idle harness. Test disabled, enabled/quiescent, visible native
view, detached job, large list and noisy helper cases, including descendants.
Measure first document output separately from first usable plugin command/view;
neither first byte nor drawing quiet proves readiness. Use at least ten startup
samples and three ten-second idle windows after settlement, serially without
concurrent compilation/coverage. Record ranges as well as medians.

Add repeatable input-to-frame latency measurements while flooding updates and
publishing maximum-size models. Target no material disabled-startup regression,
zero quiescent-plugin-induced writes, median idle CPU at measurement floor, and
under 16 ms p95 input-to-frame time on the recorded baseline machine. These are
measurement targets, not portable absolute CI thresholds. If model publication
or final text application exceeds the responsiveness budget, reduce accepted
batch sizes or move preparation off-loop before raising limits. Reuse existing
performance gating conventions and record justified hardware-dependent thresholds.

## Handoff and scope boundaries

The next session should inspect `git status`, read `AGENTS.md` and the current
references, move this plan to `active/`, and begin milestone 1. Verify source has
not changed the assumptions recorded above. Resolve routine details in the plan
as implementation reveals them; do not replace explicit targeting or bounded
ownership with a shortcut through the rendered frame.

Likely code ownership: wire/worker helpers under `src/plugin/` if splitting the
current module; host resources and adapters under `src/workspace/host/`; editor
coordination under `src/app/`; generic view projection in its own focused module;
existing buffer, text, keymap, picker, snapshot, protocol and terminal owners keep
their responsibilities. Exact new filenames may follow repository conventions.

Keep the existing [plain-pipe issue](../../issues/plain_pipe_command.md) independent.
It is useful for text filters but is not a prerequisite for application views,
providers or jobs. Do not implement it incidentally during this plan.

This plan delivers an application extension platform with native textual UI,
editor integration, remote document providers and managed external backends.
Marketplace/package management, multiple runtimes, arbitrary graphics/web UI,
inline video, cross-workspace plugin daemons, hostile-code sandboxing, general
multi-buffer atomic automation and crash-proof distributed transactions are not
part of that completion claim. None is silently required to make a file manager,
remote editor or media controller useful.
