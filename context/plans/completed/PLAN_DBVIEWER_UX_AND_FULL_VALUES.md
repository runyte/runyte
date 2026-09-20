# Database viewer actions, navigation and full-value inspection

## Status and scope

Completed, 2026-09-19. All seven working items are implemented, with independent
subagent review after each item and repeated rounds until no findings remained.
Validation below records Linux evidence; other platform and release gates remain
separate and are not claimed as passing.

Follow [the plan lifecycle](../README.md). This is follow-up work to
[database viewer interactive browsing](../completed/PLAN_DBVIEWER_INTERACTIVE_BROWSING.md)
and [stable plugin compatibility](../completed/PLAN_STABLE_PLUGINS.md).
Those completed plans preserve their original implementation and validation
records. Their outstanding platform checks are not passing evidence for this
work.

ru-dbviewer owns database reads, result retention, JSON formatting, view titles
and the choice of relevant actions. Runyte owns native buffers, presentation,
command dispatch, jobs and the public extension contract. Keep the protocol at
`runyte-1`; use negotiated features for compatible additions. Do not use private
bundled-client DTOs as the plugin API.

## Problem and intended result

The current viewer exposes an `activate` action beside `connect`, presents a
long ungrouped list of commands, and uses titles that omit the kind or source
of the displayed content. Filters appear below the rows under a generic
`Detail` heading. Routine status text describes internal retention and paging
mechanics rather than the information needed to browse.

Both adapters discard content beyond the cell's 64 KiB retention limit. Enter
opens an inspection of that shortened copy. The recent indented-prefix display
improves readability but cannot recover missing content. Complete values can
also exceed the JSON tree's node/depth limits or Runyte's native view limits.

The resulting workflow must provide:

- Enter to open an item and `-` to return to its retained parent, without an
  additional visible `Activate` action.
- Readable action labels and groups that reflect the current view and selection.
- A title identifying the view kind and its database/table/record/field context.
- Concise, explicitly labelled metadata above the content.
- Lightweight previews with `…` where content is shortened, and an explicit
  `Show full value` action that makes the complete captured value available.

No development record depends on screenshots in `.runyte/cache/images/`.
Reproduce the behavior with synthetic databases in temporary directories.

## Agreed interaction

### Commands and action discovery

Keep command identifiers and aliases within the existing lowercase
letter/digit/hyphen grammar. Spaces belong in display labels. Existing bindings,
aliases and protocol invocation continue to address stable identifiers.
For example, the existing `connect-new` identifier may remain internally while
its visible label becomes `Add database`. Preserve `::db-connect` and the other
documented `db-*` aliases; a label change does not require renaming them.

Remove `activate` from the action menu and ordinary command discovery. Enter
continues to invoke the registered primary callback. Keep any retained internal
identifier callable under its existing admission rules; hiding a menu entry
does not remove the command from the allowed-action list. Help and key hints
describe Enter by its current role, such as `Open tables`, `Open row`,
`Open value` or `Expand/collapse`, without advertising an `Activate` menu item.

On a disconnected saved profile, show `Connect` once. On a connected profile,
Enter opens its tables. `Add database` remains a separate action for creating
and connecting a profile. Pending transaction and uncertain-outcome actions
remain tied to the selected database and its actual state.

### Grouped actions

Start with labelled sections in one searchable Tab picker. Do not require
nested submenus for this iteration. Section headings are not selectable actions.
Ordinary picker navigation skips them, and Enter invokes exactly one command.
Filtering matches readable labels, descriptions and stable command spellings;
retain only groups containing matching actions and preserve a predictable order.

Suggested rows-view groups are:

| Group | Actions when applicable |
| --- | --- |
| Navigation | Previous page, Next page, Back |
| Filter and sort | Edit filters, Sort rows |
| Columns and paging | Choose columns, Page size |
| Table | Inspect schema, Refresh |
| SQL | New query, Open current browse as SQL |
| Database | Access mode, Transactions, Disconnect |

The final order should put frequent actions first and keep related actions
together. Empty groups disappear. Inapplicable actions are omitted; groups do
not grant availability. Commit and Rollback become prominent only when a
transaction is pending. Retain the existing confirmations and outcome handling.

Record/value views focus on inspection and navigation. Move routine connection
administration to database/table-level actions instead of repeating every
database command in every inspector. Database lifecycle commands remain
discoverable through their documented global aliases. This deliberately refines
the prior plan's requirement to put Disconnect in every descendant menu.

### View identity and top metadata

Use titles consistent with Runyte's special-buffer vocabulary:

| Content | Example title |
| --- | --- |
| Saved profiles | `[databases]` |
| Tables and views | `[tables] ledger` |
| Table rows | `[rows] ledger › batches` |
| Fields of one row | `[record] ledger › batches › row 1` |
| Field preview or full value | `[value] ledger › batches › row 1 › envelope · JSON` |
| SQL results | `[results] ledger › query label` |
| Schema, filters, transactions | Explicit `[schema]`, `[filters]`, `[transactions]` titles |

Use a short unambiguous row identity when available; otherwise use a correctly
numbered result/page row. Retain complete source identity internally, including
duplicate column ordinals, independent of abbreviated labels. JSON is a
detected presentation format; preserve the database's declared type in metadata.
Keep normal host-owned read-only, dirty and stale markers in their existing roles.

Place relevant facts between the pane title and content: `Database path`,
`Rows`, `Filters`, `Sort`, `Access` or a value's type/size and preview/full state.
Do not invent a total row count or value size when it is unknown. Use safe
database endpoint labels without credentials. Long paths wrap as presentation;
they are not selectable database rows. Headers never acquire row action IDs.

Stop publishing the generic bottom `Detail` section from dbviewer. Label a
filter summary as `Filters`, and a database path as `Database path`. Remove the
routine `RETAINED VALUES TRUNCATED` and `cell previews may be clipped` sentences.
Avoid filling every header with implementation warnings; offset paging's
consistency behavior remains documented in help. Loading errors and incomplete
query execution still receive explicit feedback.

### Preview and full-value behavior

Enter on a field opens its formatted preview without automatically loading its
full content into the editor. Mark shortened text with `…`, including a visible
end marker when scrolling to the end of an indented JSON prefix. Do not invent
missing delimiters or describe an incomplete prefix as a complete JSON value.

`Tab → Show full value` is available for exactly one selected field in a record
view and from that field's preview. Hide it when the complete value is already
loaded. Non-field selections and ambiguous multi-selections cannot accidentally
load another value. NULL and empty strings are already complete.

The action captures the source result, field identity, view revision and
destination. Show `Loading full value…` through a cancellable job. Success
shows the complete value in the invoking pane if foreground authority is still
valid; a later pane switch or persistent-session attachment change must not
steal focus. Reuse an existing inspector where possible. Failure or cancellation
keeps the previous preview readable and offers an explicit retry when valid.

Full inspection uses one scrollable read-only document. Normal search,
selection and copying cover the whole loaded value, including content beyond
the preview and former native-view limits. Explicit pages and page-local search
are not accepted substitutes.

Full inspection preserves parent navigation: `-` returns to the record with its
cursor and viewport restored. It does not walk through duplicate transient
loading views. Preserve ordinary buffer movement and pane behavior.

Formatted/raw switching never runs SQL. Preserve duplicate JSON keys, number
spellings, Unicode and original raw text. Separate pretty-printing from optional
interactive tree expansion: exceeding the tree-node budget must not cut off
the underlying full text. Keep existing NULL, invalid-text and binary distinctions;
binary/invalid UTF-8 values need a complete bounded textual representation, not
a silently shortened conversion presented as complete.

## Capturing complete values in ru-dbviewer

Replace the current single clipped string with a bounded preview plus explicit
information about the complete value: an immutable source handle, representation,
known size and availability. A preview's ellipsis is presentation; it does not
authorize discarding the only source from which full inspection can read.

The preferred correctness baseline is to preserve oversized values from the
original database read in private temporary storage, shared by that result's
record/value descendants. Only explicit inspection loads and formats the full
value for the editor. This distinguishes on-demand presentation from the
capture needed to preserve a result without rerunning its SQL.

Never replay arbitrary SELECT expressions or writable statements to reconstruct
discarded result values. Such SQL may be volatile or have effects, and the
underlying database may have changed. A later stable-key table fetch is a
possible optimization, but it must be explicitly identified as a fresh read and
must detect missing/changed row identity. It must not silently replace the
original result. Start with captured values for both adapters unless the initial
design audit establishes equivalent semantics for a narrower path.

Capture bytes before `Cell::new` and before the SQLite adapter's existing
blob/invalid-text shortening. Keep preview/result memory bounds independent of
full-value storage. Preserve PostgreSQL's server text representation and SQLite
type distinctions. Driver allocations for one field remain a separate constraint;
spooling after materialization is not a claim of bounded driver RSS.

Use per-owner private scratch storage with opaque handles, explicit ownership,
safe permissions and cleanup on last-reference release, stop and failed work.
Do not persist values in profiles, logs, tracked context or query history.
Specify crash cleanup without deleting another live instance's storage. Tests
must inject fixture-owned storage paths. Do not create data files in the project
or a person's configuration/cache as a side effect of inspection.

Before implementation, record concrete per-value, per-result and per-owner disk
and memory budgets, concurrent load limits, deadlines and reservation rules.
Check resource availability before promising a complete capture. Exhaustion
must report that full inspection is unavailable, not silently publish an
incomplete result as full. Preserve existing transaction rollback/retirement
rules when query execution cannot complete; failure of a later display-only job
must not implicitly commit or roll back a pending transaction.

## Runyte changes and `runyte-1` compatibility

### Action presentation feature

Proposed feature name: `view-action-presentation`. Final wire names and bounds
are fixed during the initial contract step, before implementation.

Add optional presentation metadata for command labels, discovery visibility,
groups and order. Registered command IDs remain the execution authority.
Support view-specific labels/group membership where the same command's context
changes. A primary callback can be hidden from menus and palette suggestions
while retaining Enter, configured bindings and direct invocation admission.

Resolve this metadata through shared command/view helpers used by the Tab menu,
palette, help and key hints. Do not build a second dispatch registry or infer
groups from command-name prefixes. Presentation-only changes must not bypass
row-specific action restrictions, pending-query guards, selection capture or
stale model checks.

Bound labels, group count, entries and total metadata. Reject duplicate IDs,
invalid references, control characters and unregistered commands. Charge retained
metadata to existing budgets. Preserve atomic validation for registration,
inline publication, staging and every intermediate patch operation.

If metadata is supplied during registration, send it only after the host's hello
advertises the feature and include the corresponding feature request. Validate
that relationship before installing any commands. View metadata is sent only
after the feature is acknowledged. This avoids depending on older strict parsers
to ignore unknown fields.

### Top metadata feature

Proposed feature name: `view-metadata`.

Add a bounded ordered collection of labelled values above the view's rows.
Allow independent semantic styling of labels and values with ordinary wrapping.
Integrate it into model/header patches, staging, snapshots, projection sizing
and row mapping. The metadata has no actionable database row identity.

Do not globally change the placement or meaning of the existing `detail` block.
Older plugins keep their existing rendering. New dbviewer publications use the
metadata area and omit `detail`; a legacy fallback can use a concise existing
status line without adding fake data rows.

### Full-value documents

Existing native views are limited to 10,000 rows and 4 MiB of projected text.
Staging permits larger messages but does not remove those totals. Provider
documents currently stop at 8 MiB and are editable resources, so they cannot
be assumed to provide a general read-only value inspector unchanged.

The required presentation is one scrollable read-only document whose search and
copying cover the whole loaded value. Do not switch to pages or search only a
hidden subset. Transfer may use internal chunks; these are not user-visible
pages and do not narrow editor operations to one chunk.

Audit existing asynchronous document and staged-view machinery first. Reuse
native buffer rendering, character offsets and navigation. If a new transport
is needed, add a generic negotiated read-only-document feature under `runyte-1`,
not a database-specific RPC. Design bounded chunk admission and background
preparation, exact source version/size validation, cancellation and atomic
publication. Preserve plugin ownership and the inspector's actions/parent
without inventing fake workspace filenames or writable database documents.

Chunked transfer alone does not bound final document memory. Define a supported
complete-value size and owner quota, and account for raw/formatted copies,
projection/rope storage, temporary staging and concurrent loads. If backing
storage or a larger-document architecture is necessary, specify its search,
copy, offset and lifetime behavior before coding. Never lift existing limits
silently for old clients or claim unlimited-value support.

### Compatibility and fallback

Follow [the compatibility contract](../../../docs/plugins/compatibility.md) and
[the application contract](../../../docs/plugins/applications.md). Features add
understood wire behavior; they do not grant new authority. Keep capability checks
explicit for any document operation. No change to the `runyte-1` identifier or
command/argument grammar is required.

Old plugins must run unchanged on the new host. New dbviewer code omits unsupported
fields on old hosts and uses the legacy picker/status behavior where possible.
Where full-value publication requires an absent feature, report the requirement
or use an explicitly documented complete fallback; never label its preview full.
Keep the old `connect-new` binding identity when only its label changes. Required
features and a raised minimum host release are alternatives if a usable fallback
cannot be provided. Keep generated configuration and registration ranges aligned.

Update current schemas, SDK helpers, fixtures and documentation together. Frozen
stable clients/schemas/fixtures remain unchanged compatibility evidence. An API
feature does not itself authorize a package version bump, tag, push or release.

## Implementation sequence and ownership

### Working item 1 — implementation decisions

The following wire choices refine the proposal and are reviewed before coding.

- `view-action-presentation` adds an optional command `presentation` object:
  `label` (1–160 UTF-8 bytes), optional `group` (1–64 bytes), `order` (0–65535,
  default 0) and `listed` (default true). Labels/groups reject controls. At most
  16 distinct groups occur in a registration. The same object can be supplied
  as a per-view override in `action_presentation`, a map of at most 64 registered
  view-command IDs. The override replaces that command's whole presentation.
  Group order follows each group's first command in ordered presentation;
  commands sort by order then registration order. A command absent from the
  map inherits registration metadata. `listed: false` hides discovery only.
- `view-metadata` adds optional `metadata`, at most 16 ordered `{label,value}`
  entries. Labels are 1–64 bytes, values at most 1024 bytes, with no controls.
  Both empty and nonempty authored fields require negotiation. Metadata renders
  above status/columns/rows without data-row IDs. Existing detail/preview blocks
  keep their legacy behavior. Header patches replace metadata and overrides.
- `view-document` adds optional `document` text to a native view model/header.
  Document models have purpose `document`, no rows/columns/detail/preview, and
  use the same owned view handle, read-only buffer, actions, model revision,
  parent navigation and atomic worker publication as other views. Text is at
  most 8 MiB and 250,000 lines; it permits tabs/newlines but no other controls.
  Its full body is ordinary rope text, so search, selection and copy cover the
  complete value. No row-per-line conversion or 10,000-row limit applies.
  Metadata/status may precede the body; raw content remains stored separately
  in the plugin. No implicit trailing newline is added to the body.
  Values containing forbidden controls use a complete lossless escaped-text
  representation labelled `Escaped text`; copying yields that representation.
  Literal backslashes must also be escaped in this mode so decoding is
  unambiguous. Ordinary UTF-8 raw text, including tabs/newlines, is unchanged.
  Binary and invalid-UTF-8 values use explicitly labelled complete hexadecimal
  representations. No control-containing source is silently called exact raw
  text after transformation.
- Document models use at most 16 MiB of encoded JSON and 64 MiB prepared
  payload. Staged model/patch bytes may reach 16 MiB only for owners negotiating
  `view-document`; decoded non-document models retain their original 4 MiB
  limit. Input chunks keep the existing 128 KiB limit and 1 MiB frame bound.
  Document preparation reserves 96 MiB, including staged input, preparation,
  encoded text and rope publication. A negotiated owner has a 160 MiB retained
  quota; replacement is admitted only if its reservation plus the current
  document, retained parents and other owner resources fit. Old owners
  retain 48 MiB. The aggregate host bound remains 160 MiB, so other live plugins
  can reduce available capacity and a request may fail before work begins.
  No old limit is advertised as changed to an unnegotiated client.
- Store complete plugin values in anonymous temporary files, unlinked at
  creation and held by shared handles. Use one append-only spool per result,
  with immutable byte ranges for individual values and at most 32 live spools
  per plugin. Reserve a handle before file creation and release it on failure
  and last-reference close. This bounds descriptors as well as bytes and avoids
  opening one file per large cell. This avoids crash-residue cleanup and
  never publishes a scratch pathname. Capture only when a value exceeds the
  preview budget. Keep at most 8 MiB per complete textual representation,
  32 MiB per result and 64 MiB across one plugin process. Refuse additional
  capture explicitly while keeping its bounded preview readable; no truncated
  capture is labelled complete. Last-reference release closes the file and
  returns its reservation. The storage root is injectable for fixture isolation.
- One display-only load/format job runs at a time per plugin, independently of
  SQL transaction jobs. It has a 60-second deadline, bounded control responses
  and explicit cancellation. Raw and formatted strings each fit 8 MiB. If
  pretty-print expansion exceeds that bound, show the complete raw value with
  an explanation instead of truncating formatted text. If encoded publication
  exceeds 16 MiB, refuse explicitly and retain the preview. A storage/format
  failure never settles a database transaction.
- Document publication restores non-row selections and viewport positions using
  real document line counts/body offsets, not the old row-map length. Metadata
  updates and raw/formatted replacement must not clamp positions to the header.
  Exercise replacement with retained parents and near-quota owner state.
- Formatting full JSON is lexical after validation, preserving duplicate keys
  and numeric spelling. The 4,096-node interactive preview tree remains bounded;
  full text formatting does not construct that tree. Original raw text remains
  available. Binary/invalid-text conversion is complete within the stated
  textual-representation cap, or records an explicit unavailable reason.
- Older hosts receive no new fields. They retain legacy menus and a compact
  status fallback. Full values fitting legacy views can use them without data
  loss; larger full documents report that `view-document` support is required.
  Newly captured data is never recovered by SQL replay on either host profile.

Review and test results are recorded per item below.

Item 1 completed: two independent design-review rounds, final result no findings.
The first round added lossless escaped-text semantics, a 32-spool descriptor
limit, conditional replacement admission and document position restoration.
A synthetic probe preserves a 5,615,819-byte JSON value through formatting into
48,006 lines (5,927,835 bytes), fitting the proposed document and encoded bounds.
This is a size/fidelity probe, not yet host publication acceptance.

1. **Contract and full-value design.** Inventory current APIs and capture paths;
   settle action metadata placement, exact bounds, single-document delivery,
   storage quotas and compatibility fallback. Demonstrate a complete synthetic
   value over 64 KiB and one exceeding 10,000 formatted lines. Record decisions
   here before changing public wire behavior.
2. **Host action presentation.** Extend negotiation, validation, registry helpers,
   picker grouping and presentation snapshots. Preserve unchanged clients and
   contextual dispatch. Add contract and native picker regressions.
3. **Host top metadata.** Extend models/projection and all publication paths;
   verify selectable row offsets, wrapping, snapshots and unchanged legacy blocks.
4. **Plugin UX.** Publish labels/groups, hide Activate, rename the visible Add
   database action, add complete titles/breadcrumbs and labelled top metadata,
   remove routine truncation sentences, and simplify descendant action menus.
5. **Complete value capture.** Implement immutable storage/handles in both database
   adapters, preview separation, ownership and quota/error behavior. Ensure
   arbitrary SQL is executed exactly once and all descendants share their source.
6. **On-demand full inspection.** Implement the selected host document path if
   required; wire Show full value, progress/cancellation, raw/formatted display,
   parent navigation, closed-view and attachment behavior end to end.
7. **Validation and documentation.** Update plugin guides and validation records,
   Runyte's user guide, UI vocabulary, keymap reference and public contract.
   Record actual platform results and limitations before lifecycle completion.

Runyte changes principally touch `src/plugin/application.rs`, registration
validation, `src/plugin/view/`, `src/app/plugin_views.rs`, shared runtime command
presentation, picker/snapshot rendering and the relevant workspace host modules.
Keep frontend snapshots semantic. Reuse existing document ownership rather than
making database logic part of the editor. ru-dbviewer changes principally touch
`src/app/`, `src/views.rs`, `src/inspection.rs`, `src/results.rs`, both database
adapters and a dedicated result-storage module if needed.

## Acceptance and verification

Use temporary SQLite and isolated PostgreSQL fixtures. Include duplicate column
names, empty names, NULL, empty text, Unicode crossing chunk boundaries, binary
and invalid-text representations, complete and incomplete JSON, duplicate keys,
large numbers, nested/large trees and values above each former display limit.

Required behavior coverage:

- No visible Activate entry; Enter still works at every navigation level and
  still refuses invalid/stale selections. Add database appears once with spaces;
  configured IDs, aliases and typed arguments retain their original grammar.
- Group headings cannot execute; keyboard navigation and fuzzy filtering work
  across groups. Empty groups disappear. Row restrictions and multiselection
  intersections gate the same commands before and after presentation changes.
- Titles retain database/table/row/field identity. Metadata appears above content,
  uses explicit labels, wraps correctly and never becomes a data row. Header
  updates and parent restoration preserve meaningful cursor/viewport positions.
- Previews show ellipses without the removed retention/clipping sentences.
  Show full value appears only when meaningful, from both supported entry points.
  A synthetic trailing sentinel beyond 64 KiB is reachable after loading; raw
  text matches the complete original, not merely the displayed prefix.
- A value above 4 MiB and one over 10,000 formatted lines exercise the chosen
  complete-value path. Whole-document search/copy reaches the end, including
  searches and selections crossing transport chunks. Raw/formatted switching
  preserves all data.
- An original SQL execution counter remains unchanged during opening, retrying,
  formatting, returning and reattachment. External row changes do not silently
  replace the captured value. Writable RETURNING data never causes SQL replay.
- Late results cannot retarget another buffer, pane, connection or attachment.
  Closing views, disconnecting, stopping/restarting the plugin, cancelling jobs
  and releasing the final result reference obey documented ownership/cleanup.
- Disk-full, quota, permission, malformed chunk, changed version, timeout and
  transport failures preserve the preview and leave no partial result labelled
  full. No display job silently settles a transaction or abandons an active lease.
- New fields without negotiation are rejected across registration, inline/staged
  models and patches, including transient insert/remove and update/overwrite
  cases. Old clients and unrelated plugins retain their behavior.

Run host formatting, `cargo clippy --all-targets -- -D warnings`, `cargo test`
and canonical `cargo llvm-cov --locked --workspace`; preserve the current 89%
floor on affected first-class targets. Run schema/SDK/frozen-client conformance
and unchanged ru-time/native example checks appropriate to the changed surfaces.
The [coverage register](../../reference/test-coverage.md) remains authoritative.

Run plugin formatting, locked all-target Clippy with denied warnings, locked
Rust tests, public-wire/interactive tests, both database suites and native PTY
acceptance. Preserve its 75% combined line-coverage floor and exclude old profiles
when reporting a new measurement. Every subprocess fixture owns XDG_CONFIG_HOME
and temporary runtime/storage paths; never run test-written executables.

Use the [performance register](../../reference/startup-performance.md) and
benchmark harness for new document loading or background work. Measure editor
responsiveness during a large load, cancellation, memory/storage peaks and idle
behavior after completion. Formatting and storage IO stay off the input/render
loop. No idle database polling or timer is needed for these interactions.

Native acceptance covers Databases → Tables → Rows → Record → Value, grouped
actions and filtering, full-value loading/raw switching/search/copy, both back
controls, query results, cancellation and persistent detach/reattach. Record
Linux/macOS and architecture evidence separately; missing fixtures or configured
CI jobs are not passing results. Documentation-only planning checks links and
diff hygiene without claiming these implementation tests have run.

## Scope boundaries

This work does not add database cell editing, automatic full-value opening,
automatic SQL replay, background database polling, durable query/value history,
arbitrary scripting, a new command language, or a new protocol epoch. Preserve
ordinary editor keys, explicit writable execution/settlement and existing
connection-generation checks. General nested action menus and an unlimited-size
document engine are not prerequisites; any larger document architecture must
be justified by the selected full-value design and remain bounded.

## Implementation progress and review record

- Working item 1: design settled; independent review rounds resolved lossless
  control-character display, spool-handle limits, document-position restoration
  and quota admission details. Final review: no findings.
- Working item 2: host command labels, hidden discovery, grouped action rows,
  contextual help/hints and private frontend DTO version 54 implemented.
  Review found missing registration-presentation quota accounting and authored
  null handling; both corrected. Repeat review: no findings. Focused action,
  section navigation and registration boundary tests pass.
- Working item 3: top metadata and per-view action presentation implemented.
  Independent review: no findings. Model and host admission tests pass.
- Working item 5: original SQLite/PostgreSQL values captured into bounded
  anonymous result spools. Review found capture work could bypass SQLite's
  deadline; cooperative cancellation/deadline checks now cover capture/read
  chunks and final completion. Repeat review: no findings. Rust tests and an
  isolated PostgreSQL 17.9 capture/transaction/cancellation fixture pass.
- Working item 6 (host): negotiated complete documents, staged publication,
  larger preparation reservations and ordinary-buffer positions/search/copy
  implemented. Independent review: no findings. Focused tests cover a >4 MiB,
  48,000-line document, quota refusal preserving the previous view, and native
  whole-document search/copy. Plugin integration and end-to-end validation are
  complete, as recorded below.


### Additional job-feedback decision

Integration review identified a missing background-error path: a full-document
raw/formatted toggle can fail while the existing complete document must remain
visible. Replacing its header republishes the body and can hit the same quota;
foreground input surfaces require a live invocation; notifications require a
separate grant. The implementation adds negotiated `job-feedback` to `runyte-1`:
`job.finish` accepts an optional 1–1,024-byte, control-free message. The host
reserves this capacity when creating a negotiated job, retains the actual reason
with the result, releases unused capacity, and uses native job feedback without
retargeting the user's later work. Older messages omit it unchanged. ru-dbviewer
requires both `view-document` and `job-feedback` before offering complete-value
loading; older hosts keep previews. Independent review of wire gates, accounting,
SDK/schema, ownership and native feedback completed with no findings.

Working items 4 and 6 (plugin) now passed independent review rounds. Corrections
include preserving nondefault page sizes only in retained-result descendants,
qualifying PostgreSQL schema identities, stable value breadcrumbs after failures,
closing staged preparation on cancellation, cleaning refused view setup, and
returning full values directly to the record. Native large-value assertions
verified formatted and raw copying, search and persistent detach/reattach;
final reruns and validation inventory follow.

### Completed validation

Working item 7 completed with independent documentation and validation review;
no findings remain. These results are local Linux x86-64 evidence, using
Rust/Cargo 1.97.1, Python 3.14.7 and current debug builds of both repositories.

- Runyte: formatting, all-target Clippy with denied warnings and `cargo test`
  passed. Canonical `cargo llvm-cov --locked --workspace` passed 3,795 tests
  across 37 suites, with 34 ignored, and measured 91.91% line coverage
  (125,130/136,147), above the unchanged 89% floor.
- Contract checks passed: 12 schema, 16 model, nine compatibility and 11
  application checks; unchanged frozen clients and fixtures; pinned ru-time's
  65 tests and two strict native cases; all 19 bundled example programs in a
  real host. Current examples do not replace frozen-client compatibility.
- ru-dbviewer: formatting, locked all-target Clippy with denied warnings,
  41 ordinary Rust tests, 17 public-wire tests, 38 interactive tests, nine
  complete-value tests and eight native PTY cases passed. All five PostgreSQL
  suites passed against isolated PostgreSQL 17.9, including TLS, client
  certificates, Unix sockets and lost-commit acknowledgement.
- Fresh combined plugin LLVM profiles from those suites measured 89.90% line
  coverage (5,652/6,287), above its unchanged 75% floor.
- The plugin's `cargo +1.88 check --locked --all-targets` passed. Its normal
  uninstrumented debug executable was rebuilt after coverage, and locked Rust
  tests passed again.
- Native acceptance covers complete formatted/raw copying, tail search, direct
  return to the record and detach/reattach. The uninstrumented performance
  observation and its measurement limits are recorded in the
  [performance register](../../reference/startup-performance.md); host coverage
  is recorded in the [coverage register](../../reference/test-coverage.md).

An instrumented cancellation run exposed an intermittent plugin shutdown. The
subsequent transport audit found a real request-ordering race: concurrent large
document frames and small cancellation requests could queue increasing request
IDs in reverse order. Request allocation and queue insertion are now serialized,
without holding that lock across socket IO or awaiting a reply. A concurrent
128-request regression passes; independent review found no remaining findings,
and the complete fresh-profile validation above passed after the correction.
The race is source-confirmed, but was not established as the cause of that
particular shutdown.

Native macOS, ARM64, packaged artifacts and release CI were not run. No release,
publication or installation into the user's executable directory is claimed.
