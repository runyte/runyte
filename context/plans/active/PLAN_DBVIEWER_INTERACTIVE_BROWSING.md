# Database viewer interactive browsing

Status: implementation delivered; automated Linux acceptance recorded below.
Native macOS/ARM64 and separate human manual acceptance remain outstanding.
Date: 2026-09-19.

## Objective

Make ru-dbviewer discoverable through native forms, contextual actions and
predictable parent navigation. Support useful database exploration without SQL,
while retaining ordinary editable SQL buffers for more complex work. Preserve
captured execution, bounded results, explicit writable review and transaction
ownership.

This plan coordinates the independent ru-dbviewer plugin and any necessary
Runyte capabilities. Plugin changes belong in the sibling plugin repository;
generic editor interaction changes belong in Runyte. The public runyte-1
contract is the integration boundary. Do not use private bundled-client DTOs.
This document authorizes the agreed scope, not publication or release.

## Implementation record

The plugin implementation and its first review loop are recorded in
ru-dbviewer `INTERACTIVE_BROWSING.md` and `VALIDATION.md`. The subsequent host
extension adds optional `view-row-actions` within `runyte-1`: negotiated row
lists override model-wide actions, and selections spanning several data rows
use their intersection. Availability gates both discovery and dispatch.
Inline and staged patches validate every row action list before operations can
remove it. Existing plugins negotiate no feature and keep their original menus;
ru-dbviewer retains its profile-actions fallback on older hosts.

The public contract is [Row-dependent actions](../../../docs/plugins/applications.md#row-dependent-actions).
Connection forms/completion, query documents, browsing, JSON and transaction
handling remain plugin-owned. No publication or release is part of this work.
The coverage register and plugin validation record distinguish actual Linux
execution from remaining platform acceptance.

## Observed friction

- The backend choice and subsequent connection form look too similar. It is
  unclear that profile name and database path are both required inputs.
- Database paths lack familiar resolution feedback and completion hints.
  Workspace-relative paths already work, but this is not discoverable.
- Enter drills down, but returning requires discovering contextual actions.
  Returning from the table catalog to the database list is also unclear.
- The database list omits access-mode selection, although it identifies the
  connected database. Recovery acknowledgement appears without useful context.
- Action names expose a view- prefix, and transaction actions can appear to do
  nothing when no transaction is pending.
- SQL buffers have opaque timestamp names and do not teach execution, saving,
  connection association, or return navigation.
- Page size, column selection and row filtering need more interactive controls.
- JSON is displayed as a long line. Preview clipping and retained-value
  truncation are difficult to distinguish.
- Pending transactions need a discoverable overview and direct settlement.

## Agreed interaction model

### Connection form and paths

Present the SQLite connection overlay as a form with two clearly labelled,
required fields: Profile name and Existing database path. Distinguish editable
fields from a choice list. Show field movement and submission instructions using
the actual supported bindings. Validate each field and keep entered values when
validation fails.

Reuse Runyte's path resolution and completion semantics where applicable. Audit
the existing gf and path-prompt implementations before specifying expansions;
document the base directory explicitly. Support absolute and workspace-relative
paths, show the resolved destination, and offer directory/file completion.
Completion must be bounded and must not block editor input or accept stale
results after the field changes. Opening still requires an existing SQLite file.
Do not execute shell expansion to resolve a path.

### Contextual action names and availability

| Current action | New visible action | Description |
| --- | --- | --- |
| view-connect | connect-new | Connect to a new database |
| view-query | query | Open a SQL query buffer |
| view-commit | commit | Commit pending changes |
| view-rollback | rollback | Discard pending changes |

Resolve the target from the selected profile or owning view, never from a
different database that became active later. Retain documented db-* command
aliases. Audit command identity collisions before renaming registrations; if
compatibility aliases are retained, avoid duplicate entries in visible menus.

Only expose recovery acknowledgement for an actual unresolved recovery marker.
Describe it as Acknowledge uncertain outcome and explain the required independent
database review. Acknowledgement must not imply commit, rollback, or SQL replay.
Commit and rollback must have clear unavailable-state feedback or be omitted
when irrelevant. No successful-looking silent no-ops.

### Parent navigation

Use Enter to open the selected item. Provide both - and Tab back to return:

```text
Database list -> Table catalog -> Table rows -> Record -> Value
```

Returning from the catalog reaches the database list without disconnecting.
Restore the parent cursor, viewport, page, selected columns, filters and sorting.
Use explicit parent identities rather than global navigation history. Bound
retained navigation state and handle closed parents or disconnected databases
with a meaningful fallback. Returning to retained results must not rerun SQL.

Keep ordinary SQL buffers editable, including normal Tab insertion. Provide a
discoverable return-to-source action for query buffers without taking over
ordinary text keys. Document its final command spelling in generated comments.
Any host binding additions must remain in the shared keymap registry and help.

### Access mode

Offer mode from both the database list's selected connected profile and the
table catalog. Open a menu with exactly READ ONLY and READ AND WRITE and mark
the current choice. Selecting the current mode is a no-op with clear feedback.

Switching to READ AND WRITE requires confirmation. Switching to READ ONLY does
not require a mode confirmation. If a transaction is pending, require explicit
commit or rollback before changing modes; do not silently discard it. Cancelled
settlement leaves the connection unchanged. Preserve the existing reconnect and
connection-generation rules, and explain that existing SQL buffers require
db-use after a successful mode change. Handle busy or failed reconnects clearly.

### Disconnect

Add Tab disconnect to every relevant menu associated with a live connection:
the selected connected database profile, catalog, browse results, record/value
inspection, query results, and that database's pending-transaction entry. Include
it wherever contextual query-buffer actions are supported. Do not expose a
misleading disconnect action for an already disconnected profile.

Capture the exact connection identity and generation. Preserve the existing
pending-transaction protection: explain that disconnect will roll back pending
changes and require confirmation; cancellation leaves the connection intact.
For a running operation, provide explicit cancellation guidance rather than
silently terminating or retargeting it. Disconnect invalidates SQL associations
and updates all affected views; retained data may remain readable but must show
that the connection is disconnected. Back navigation alone never disconnects.

### Query documents and filenames

Create named, unsaved SQL buffers using:

```text
<databasename>-query-<datetime>.sql
ledger-query-20260919-094512.sql
```

Use the database profile name and local date/time. Sanitize filename-unsafe
characters, path separators and empty names. Avoid collisions with both open
buffers and existing files, including rapid creation and clock changes; prefer
additional timestamp precision while retaining the agreed pattern. Never
overwrite an existing file as a side effect of creating a query buffer.
Creation does not write a file; :w explicitly saves it.

Begin every new query buffer with concise SQL comments explaining:

- The associated profile and access mode at creation, with a note that mode can
  change; do not imply that editable comments control execution targets.
- Write one supported SQL statement, and use ::db-run to execute the entire
  currently focused buffer, including unsaved edits.
- ::db-run-selection executes one nonempty selection.
- Each query buffer has its own database association. ::db-use associates an
  ordinary buffer or reassociates it after reconnect/mode change/plugin restart.
- :w saves SQL text to a file; saving is optional and never executes SQL.
- How to return to the source browsing view and reopen database browsing.
- Writable execution requires captured review and confirmation, then explicit
  commit or rollback; rollback discards uncommitted changes and cannot undo a
  commit. Query cancellation is a separate action.
- Pending changes expire under the existing idle rollback policy.

Keep the comments short enough to leave useful editing space. Verify they remain
valid input to the one-statement parser. An editable comment must never be used
as authoritative connection state. Preserve captured text/revision/generation
checks when users switch buffers, edit SQL, or change database focus.

### Interactive browsing

Show the visible row range and configured page size (currently 100), including
empty and partial pages. Do not claim a total row count without querying it.
Retain the warning that external writes can shift offset pages. Distinguish
database browse paging from paging through retained SQL results.

Replace ordinal-only column selection with a searchable checklist of column
names. Preserve selections while searching; duplicate and unnamed columns must
remain distinguishable by stable ordinal identity. Respect display limits and
provide clear feedback when too many columns are selected.

Provide a filter editor with repeated Add filter operations. Each condition
chooses a column, a type-appropriate operator and a value. Support editing,
removing, temporarily disabling and clearing conditions. Offer Match ALL (AND)
and Match ANY (OR) for the enabled list. Keep active filters visible above rows.
Applying changes starts at page one; subsequent pages preserve the filters.
Nested Boolean groups initially remain a SQL use case.

Include equality, contains where meaningful, ordered comparisons and NULL
checks. NULL operators take no value. Define conversion errors, empty-string
behavior, literal versus wildcard contains semantics, and SQLite's dynamic
typing explicitly. Bound condition counts and value sizes. Use quoted validated
identifiers and bound parameters in both database adapters; user input must not
be concatenated as executable SQL. Disabled conditions do not affect execution.

Add interactive sorting with deterministic tie-breaking where suitable keys
exist. Open current browse as SQL generates editable SQL reflecting selected
columns, enabled filters and ordering, with an explicit bounded page/limit policy.
It must not execute automatically. Because arbitrary SQL currently does not
prompt for unbound parameters, generated SQL must be independently runnable
using tested dialect-specific literal rendering, or an explicitly designed
binding mechanism. Do not introduce unresolved placeholders silently.

### JSON inspection

Detect complete JSON in retained text values and offer formatted multiline
inspection with indentation, collapsible objects/arrays, and a raw-text option.
Preserve original text and numeric precision; formatting must not mutate stored
values or normalize large numbers through floating point. Bound parsing depth,
rendered output and tree state. Keep record previews compact and value
inspection useful for nested content.

Distinguish display-preview clipping from retained-data truncation. A truncated
value must show an incomplete-value explanation and readable retained text;
never present it as complete JSON or attempt to reconstruct missing content.
Preserve control-character escaping and current binary/NULL distinctions.

### Pending transactions

Add a discoverable Transactions view, reachable from database actions and a
documented command. Show each pending transaction's database profile, state,
age, bounded statement summary and affected-row count when known. Unknown counts
must remain unknown. The current limits remain one pending transaction per
connection and at most two live connections.

Tab commit and Tab rollback settle the selected transaction, with stale-entry
and connection-generation checks. Update the list after settlement, disconnect,
failure and automatic idle rollback. Preserve activity leases, shutdown
protection and uncertain-outcome recovery; never replay SQL. Do not persist SQL
values or summaries in profile state, logs or development context. Avoid adding
database polling or an unnecessary timer merely to display age.

## Implementation sequence and ownership

1. Audit public API support for path fields/completion, multi-selection choices,
   scoped back bindings, parent restoration and query return actions. Map plugin
   changes across app commands/input/work/lifecycle, views, query construction,
   results and both database adapters. Record concrete API gaps before extending
   the host. Inspect stable-plugin compatibility work before changing wire values.
2. Implement action naming/availability, mode selection, disconnect and parent
   navigation as one coherent browsing foundation.
3. Improve forms and path handling. Any host capability must be generic, bounded,
   documented and compatible with the stable protocol's negotiation rules.
4. Add query filenames, introductory comments and source navigation. Verify
   unsaved execution and explicit save behavior end to end.
5. Implement browsing state, column selection, multiple filters, sorting, page
   feedback and generation of runnable SQL.
6. Implement bounded JSON inspection and pending-transactions navigation/actions.
7. Update plugin README and validation records; update Runyte user guide,
   keymap/UI references and public contract documentation for host changes.
   Complete native acceptance before marking this plan complete.

## Verification and acceptance

Use temporary SQLite databases containing ordinary rows, duplicate/unnamed
result columns, mixed types, NULL/empty strings, quoted identifiers, complete
nested JSON, large integers and truncated values. Use isolated PostgreSQL
fixtures for dialect behavior. Never exercise writes against personal databases.

Behavior tests must cover:

- Required form fields, path completion/resolution, stale completion rejection,
  invalid paths and bounded responses.
- Exact action names, target ownership, mode choice/confirmation, pending-mode
  settlement, conditional acknowledgement and disconnect in each relevant view.
- Enter/-/back round trips preserving browsing state without SQL replay, plus
  closed-parent and disconnected-view behavior.
- Multiple query buffers targeting different connections; unsaved edits, explicit
  saving, generation invalidation, filename sanitization and collision handling.
- Comment-bearing query parsing, selection execution and source return.
- Multiple ALL/ANY filters, disabled/edited conditions, NULL semantics, malicious
  identifier/value inputs, sorting and filter preservation across page changes.
- Generated SQL matching the browsing selection/filter/order semantics on both
  adapters, with no accidental execution.
- JSON precision, collapse/expand/raw behavior, malformed/deep/large input and
  separate preview-versus-retention truncation indicators.
- Transaction selection, commit, rollback, expiry, disconnect confirmation,
  stale actions, busy connections and uncertain commit outcomes.

Run plugin formatting, locked all-target clippy with denied warnings, locked
tests, public-wire tests, affected database suites and native Runyte PTY tests.
Preserve the plugin's 75% combined coverage floor. For Rust host changes run
cargo fmt --check, cargo clippy --all-targets -- -D warnings, and cargo test;
preserve the canonical cargo llvm-cov --locked --workspace baseline on affected
first-class targets. Read the startup-performance register before adding timer
work. Native subprocess fixtures must own XDG_CONFIG_HOME and all runtime state.
Report actual platforms and checks; configured CI is not passing evidence.

Manual acceptance follows the whole flow: connect using completion, browse and
return with both controls, apply several filters, inspect JSON, open generated
SQL, execute unsaved text, switch mode, inspect/settle a pending transaction,
disconnect and reconnect. All critical actions should be discoverable without
external instructions.

## Scope boundaries

Keep result/SQL/view limits, one-statement execution, explicit SQL review and
connection-generation safety. This work does not add editable result cells,
multiple-statement scripts, automatic commits, automatic SQL replay, nested
filter groups, durable query history or automatic retrieval of truncated values.
Exact host API additions and the return/transactions command spellings are
implementation design details to resolve during the initial audit, not reasons
to silently omit agreed behavior.
