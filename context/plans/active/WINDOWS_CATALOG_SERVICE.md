# Native control CLI and workspace service integration

The native control CLI, typed row identity and owned catalog service are now
implemented and independently reviewed; see the
[continuation checkpoint](../../reviews/windows_phase2_handoff.md) for the
accepted commits and validation. This record retains the design decisions and
acceptance boundaries. Native manager controls are accepted; public attachment,
foreground host supervision and parent-terminal authorization remain later work.

## Recommended implementation order

1. Native catalog actions and CLI control, keeping exact selected publications.
2. Shared row-selection identity and presentation extraction, mechanically
   preserving Unix behavior.
3. Owned native background service, followed by enabling the existing session
   manager's supported control actions. Attachment/destination visits remain
   explicitly unavailable until WP5 accepts their native transport/frontend path.

Each package needs its own review and native acceptance. Do not change all Unix
cfg gates in one pass: several currently guard both presentation and authority.

## Exact row selection

The current shared WorkspaceRow is a display value, not publication authority.
Native CatalogEntry retains Candidate copies and an actual peer handle, while its
workspace ID and project root can legitimately match a second live publication.
Keep that distinction through every manager action.

Prefer a platform-neutral optional opaque publication key in WorkspaceRow, plus
a shared WorkspaceSelection value computed from the row. Unix and stopped rows
use project identity; native live rows include the opaque publication key. The
key represents the full native publication tuple (exact project bytes, process
identity, incarnation and pipe address), not a display name or workspace ID.
Its representation can be a private-constructor fixed-size digest with explicit
field framing. It is a lookup identity, never permission to operate on a process.
Keeping the field shared avoids cfg-dependent UI logic; existing Unix row literals
gain None mechanically, with behavior parity checks and tests. A Windows-only
field would reduce initial literals but spread platform branches through shared
selection and preview code, so is less attractive.

Separate service targets explicitly:

- UserSelector { selector, working_directory }: resolve against a complete new
  catalog, preserving ambiguity errors.
- SelectedRow(WorkspaceSelection): look up that exact current/retained entry;
  refuse a stale key. Never retry using its project path, old name or PID.

Keep Candidate/proof ownership in the service. Before releasing/replacing its
current snapshot, move an admitted request's selected entry into its own bounded
operation state (or retain an Arc of the selected snapshot). An in-flight request
must retain its original key/proofs even when a newer refresh replaces visible
rows. If a selected key has already expired before admission, return stale
selection; do not keep an unbounded historical map. Publication replacement must
produce a new key even if name, project and PID text appear unchanged.

Current UI path assumptions requiring conversion together:

- workspace_workflows.rs Polled restores selection using project_root; use the
  row selection instead, so two same-project rows stay distinct.
- workspace_previews and workspace_preview_target in app.rs use PathBuf keys;
  use WorkspaceSelection, and carry that identity through Previewed completion.
- Selected-row stop/rename/inventory and delayed prompt confirmation currently
  capture only a path; capture the row selection at intent creation.
- WorkspaceEvent preview/stop/rename correlation currently carries a path;
  preserve display paths but correlate by the typed selection and generation.
- Current-session highlighting/number matching, cycling and destination state
  also match project paths. Native persistent frontend integration must supply
  its own publication identity before those matches are enabled.
- Snapshot/picker identities must distinguish rows when serialized for a future
  attached frontend. Do not enable native attachment merely to render this UI.

History numbering, forget and stopped-name operations remain project-scoped.
Do not silently assign one project's history digit as an unambiguous shortcut to
two live native publications. Follow the accepted history package's duplicate
policy; if it cannot choose, refuse the action or leave the shortcut unavailable.

## Project-independent discovery scope

Selector-only CLI modes must not construct a dummy current project from cwd.
Current snapshot(ResolvedLayout, ...) requires a project and would otherwise
create that accidental dependency. Extract the existing captured namespace
selection into a frozen DiscoveryScope containing admitted cache/runtime choices,
namespace roots, explicit inventory choice and reserved/configured state inputs.
ResolvedLayout composes this scope with a real project/endpoint/name-store address;
both paths use the same selection logic and existing launch fingerprint fields.
Do not duplicate fallback resolution or change fingerprint serialization.

Native catalog observation accepts that scope plus zero or more explicitly known
ready locations. A real current workspace may contribute one; selector-only CLI
contributes none and adds only exact remembered/configured ready locations. Hidden
inventory remains optional and never reconstructs known ready/name-store paths.
This lets list/stop/rename operate when cwd is outside a project or gone, without
inventing startup authority. Test scope/layout parity, preserved explicit absence,
missing cwd, no dummy .runyte creation, and unchanged launch mismatch rejection.

## Native control actions and CLI

Add a narrow Windows control orchestration module shared by CLI and service;
keep catalog discovery, native transport and history in their existing owners.
It accepts captured configuration/layout/history inputs and typed targets. The
CLI captures roots once, resolves no invented project for selector-only modes,
and preserves explicit root absence/values using the location resolver.

List uses the accepted complete catalog/history result. Extract only the pure
table projection/printing from main.rs list_sessions and reuse it for both
platforms. Keep current columns and order; duplicate publications stay separate.
An incomplete snapshot fails rather than printing a successful empty/stopped list.

Stop resolves exactly once, retains the chosen entry, and reconnects using its
exact metadata so the actual server identity is checked again. Compatible normal
stop uses shutdown_host; compatible force uses force_shutdown_host. Await the
returned receipt with await_host_stopped before claiming completion. An
incompatible host needs explicit force and terminate_incompatible_host on a
retained Candidate, whose implementation authenticates the actual pipe peer.
Never turn a generic connect/read/timeout failure into incompatible force or
launch permission. Cleanup uses only remove_stopped_observation for matching
retained Candidates; owner-inventory rows do not authorize inferred ready paths.

Stop-all first obtains one complete catalog and then attempts every distinct live
publication despite individual refusal. Retain exact entry identity per attempt,
bound the list by existing catalog limits, and aggregate bounded failure summaries.
Use a small fixed concurrency only if needed; sequential execution is the simpler
first implementation. Cancellation may stop remaining admission but must report
which admitted outcomes are unknown; it is not rollback. No unbounded output or
one-task-per-row fan-out.

Rename uses rename_host for a live selected publication and the accepted history/
NameStore operation for a proven stopped project. A timeout or caller cancellation
after sending rename is an unknown outcome, not permission to alter stored files.
Clean follows accepted history cleanup only; it does not erase indeterminate live
records or broaden Candidate cleanup authority. Restart remains separately gated
until the exact stop-to-start transition and detached launcher are accepted by
real-host tests; do not reimplement it as catch-any-error-and-spawn.

## Background ownership

Do not copy Unix WorkspaceService's detached tokio::spawn tasks or generic
ServiceLane's detached std thread into the native lifetime contract. Use a small
native owner with one dedicated thread/current-thread Tokio runtime. That thread
creates and uses every native client and runs synchronous bounded filesystem
work, keeping it off editor rendering/input.

Suggested split:

- WorkspaceServiceHandle: clonable bounded request sender and latest-preview
  watch sender only; stored in App, never owns a joining destructor.
- WorkspaceServiceOwner: stop signal, retained completion receiver and thread
  JoinHandle; stored in HostServices, with async shutdown and joined Drop fallback.
- WorkspaceEvent receiver: bounded semantic completions (existing capacity 16),
  preserving generations and new typed selection correlation.

Keep request capacity 16 and a latest-only preview slot. Validate name/path/target
sizes before cloning/admission; a count bound alone does not bound owned bytes.
One mutation/request runs at a time. Preview may run as one owned cancellable
future beside it on the same runtime; superseded previews are discarded by their
generation/selection. No detached per-request tasks. Snapshot/proof retention is
bounded by current snapshot and the fixed number of admitted operations.

Stop admission first. Shutdown cancels read-only IO/preview and stops new queued
work. An already begun synchronous history/name mutation finishes its owned
transaction; do not abandon its ledger. Every event publication selects against
shutdown so a full/abandoned event queue cannot prevent joining. Cancelled shutdown
must retain its completion receiver and join handle for a later retry. Normal
shutdown waits completion, then joins once; Drop signals and joins as fallback.
Native synchronous filesystem calls are not forcibly interruptible: promise bounded
queues/scan sizes/IO deadlines and joined ownership, not a hard wall-clock join.
No blocking join from the ordinary App rendering path.

Missing Git is a supported state. Directory-worktree discovery must use
GitCliProvider::from_environment and return its supported empty/unavailable result,
not copy Unix catalog's GitCliProvider::new("git") unconditional invocation.

## Concrete integration map

- src/workspace/catalog_values.rs: shared row selection values/event correlation;
  existing pure formatting/history helpers stay shared.
- src/workspace/windows_catalog.rs and new Windows actions/service module: exact
  selected-entry map, bounded operations and owner.
- src/workspace/mod.rs: platform service aliases/reexports only after each owner
  is accepted; remove temporary dead-code allowances when actually consumed.
- src/main.rs: selector-only CLI mode dispatch near run's early lifecycle branch;
  pure list formatting helpers; HostServices and start_host_services; all relevant
  standalone/host common cleanup paths must await native owner shutdown.
- src/windows_host.rs: native Workspace service events and explicit owner cleanup;
  retain intentionally disabled attachment/physical-input paths.
- src/workspace/host.rs: Workspace HostEvent cfg and dispatch.
- src/app.rs and src/app/workspace_workflows.rs: service port, shared row state,
  previews/actions/selection identity and narrowly opened presentation gates.
- src/app/tests/workspace.rs, session_navigation.rs; snapshot.rs and protocol/frame.rs:
  existing manager behavior plus exact row identity through presentation.
- README.md and docs/user-guide.md: document accepted native controls only, and
  preserve unavailable attachment/parent restrictions until their own acceptance.

## Acceptance boundaries

Native real hosts: list/rename/normal protected stop/force stop; malformed or busy
candidate fails complete refresh without history mutation; incompatible actual
peer authentication; same project in two isolated namespaces stays two rows and
name/path/ID ambiguity refuses destructive actions; one stop-all refusal does not
prevent unrelated clean host completion.

Replacement races: selected row survives a newer display refresh as the same
publication; if replaced, its old key cannot rename/stop/preview the new host.
Include pending prompt confirmation, preview response and a disconnected rename
caller with unknown outcome. Missing-project hosts retain captured identity.

Service fixtures: request/full-event backpressure, latest-preview coalescing,
closed receivers, idle shutdown, cancellation during connect/read, shutdown during
held synchronous mutation, cancelled shutdown then resume, worker panic and joined
Drop. Prove fixture storage is removed only after worker/client owners are gone.

Shared UI tests: duplicate project rows retain separate selection/previews and
stale completion rejection; Unix project-only behavior is unchanged. No filesystem
or process IO runs from App event/render methods. Missing Git creates no process
attempt/error loop. Public attachment still refuses until WP5 acceptance.
