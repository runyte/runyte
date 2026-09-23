# Windows Phase 2 continuation

Checkpoint: 2026-09-23, branch `feat/windows-support`, private native
ParentAttach is accepted in `42f2c9c`, durable Windows plugin state is accepted
in `140347e`, private native parent waits are accepted in `fa18a52`, and private
Windows plugin handoffs are accepted in `8148437`. Private
exact native switching is `11963b1` and the native plugin worker foundation is
`ce888ee`. Typed switch intent and exact native target preparation are
`8657281` and `ec33c26`. Private native frontend attachment is `180175d`.
The macOS host queue EINTR repair is `dad1d86`; Unix plugin fixture readiness
repairs are `26f6c4e`, `333771d` and `0adcf38`. The preceding native host
attachment is `1e69755` and shared response ordering repair is `5d727db`.
This record supplements the [active plan](../plans/active/PLAN_WINDOWS_PHASE2.md)
with the working-tree state and immediate continuation steps. Read this record
before the older chronological progress entries. No previous chat is required.

## Scope and delivery

Phase 1 is complete. Phase 2.1 through 2.4 and the native Ctrl+h/Ctrl+j
correction are complete. Phase 2.5 has accepted private ParentWait and
ParentAttach paths; public availability remains gated. Phase 2.6 has accepted
native worker, durable-state and private handoff foundations, with context access
and the bridge still pending. Integrated Git is optional: missing
Git must leave the integration disabled without failed spawn loops or runtime
errors. Combined branch/worktree deletion still needs the native
persistent-session coordinator; separate guarded operations work.

Continue sequential work packages with an independent subagent review after
each package. Incorporate findings and repeat review until none remain before
advancing. Commit and push accepted checkpoints to `feat/windows-support`.
The user authorized pushing accepted checkpoints to `feat/windows-support`,
but not to other branches. The exact push through `d9361e6` to
`git@github.com:runyte/runyte.git` succeeded, and the later push through
`777dbf3` succeeded; accepted later checkpoints may be pushed
to that branch after review and local validation.
Report completion of the entire Phase 2 or an unexpected blocker requiring a
decision. Do not enable public persistent
attachment, plugins, or context access merely because their foundations exist.

## Accepted implementation and validation

Commit `221a48d` contains exact native live-publication discovery, remembered
history decoration, and guarded history updates. It passes formatting,
all-target Clippy with warnings denied, and the full native workspace suite:
3,254 passed, zero failures, 58 ignored entries across 45 libtest/doc-test groups,
plus six native LSP transport cases. All jobs pass in
[CI run 35655164347](https://github.com/runyte/runyte/actions/runs/35655164347),
including native Windows, Linux/macOS acceptance, and both unchanged 89% Unix
coverage gates. Windows coverage remains provisional; do not claim a measured
native llvm-cov baseline.

The previous checkpoint `dd54f43` and its CI run `35651437666` also pass. Its
preceding fixture repair `249173f` confines an actual invalid-UTF-8 filename test
to Linux and adds an actual Unicode OpenBuffers test on every platform. Unix
raw-byte codec coverage remains enabled; do not restore the invalid filename
filesystem fixture on macOS.

## Accepted package: 4e.3e discovery scope

Six source files contain the independently reviewed DiscoveryScope package:

- `src/workspace/windows_location.rs` and its `tests.rs`;
- `src/workspace/windows_catalog.rs` and its `tests.rs`;
- `src/workspace/windows_catalog/history.rs` and its `tests.rs`.

`DiscoveryInputs` and `DiscoveryScope` freeze runtime/cache/namespace choices
without inventing a cwd project. `ResolvedLayout::from_scope` composes an actual
project with those choices. Fingerprint framing and detached environment changes
remain equivalent to the preceding implementation. New scope-based catalog and
history APIs accept an optional current ready address; their ready observations
do not authorize cleanup. Existing layout APIs remain compatible.

Final independent review has no findings. All six files were compared with the
reviewed snapshots before acceptance and still match. Fifteen location tests and
34 catalog tests pass, including eight new scope regressions. Final formatting,
all-target Clippy and the complete native workspace suite pass: 3,262 tests,
zero failures and 58 ignored entries across 45 libtest/doc-test groups, plus
six native LSP transport cases. This package is delivered in `ab4a0a3`; the
all-green CI result above belongs to `221a48d`.

Local diagnostic logs are `target/windows-discovery-scope-location-tests.log`,
`target/windows-discovery-scope-catalog-tests.log`,
`target/windows-scope-checkpoint-clippy.log`, and
`target/windows-scope-checkpoint-tests.log`. They are disposable evidence, not
required development records.

## Accepted package: 4e.3f authoritative stopped names

The [reviewed stopped-name design](../plans/active/WINDOWS_STOPPED_NAMES.md)
contains the authority, ordered locks, bounded recovery and acceptance contract.
Its implementation is committed in `8d5c009`. All four source replacements
matched their `.before` baselines, allowing only line-ending differences, and
all eight applied destination files matched the reviewed staging bytes.
Independent source review found no actionable findings.

The package has four replacements, each with a `.before` comparison baseline:

| Staged replacement | Source destination |
| --- | --- |
| `names.after` | `src/workspace/windows_endpoint/names.rs` |
| `endpoint.after` | `src/workspace/windows_endpoint.rs` |
| `history.after` | `src/workspace/windows_catalog/history.rs` |
| `catalog_values.after` | `src/workspace/catalog_values.rs` |

Four new files live under the staged `src/` tree, with identical source paths:

- `workspace/windows_endpoint/names/stopped.rs`;
- `workspace/windows_endpoint/names/stopped/tests.rs`;
- `workspace/windows_catalog/history/stopped_names.rs`;
- `workspace/windows_catalog/history/stopped_names/tests.rs`.

The existing history transaction module was left unchanged. The ignored
staging directory is disposable; the retained design and committed source
define this package now.

The sixteen staged fixtures cover noncreating stored-name reads; stored/live/cache
precedence and default reservation; exact live and hidden metadata; stale intent,
forgotten history, configured collisions and startup serialization; exact occupied
or malformed records; committed rename despite cache failure; foreign replacement,
source chains, retained recovery, partial staging/restoration and owner teardown.
Review corrections preserve the original native error source, reject a changed
cache fallback when no stored authority exists, gate the now-unused shared helper
to Unix/tests, and actually persist fixture names before testing authority.

The two focused filters passed 11 and five tests. Existing endpoint and catalog
filters passed 15 and 13 tests, with one compiled endpoint fixture ignored.
Formatting, all-target Clippy with warnings denied, and the complete native
workspace suite passed: 3,288 tests, zero failures, 58 ignored across 46
libtest/doc-test groups, plus six native LSP transport cases. The sandbox token
denied private-storage fixture setup; the normal-token run passed. Native CI
acceptance for this commit is pending. Keep the Windows support issue open until
the entire scope is resolved; issue resolution requires a separate follow-up
commit.

## Accepted package: 4e.4a native control actions

`443eb7b` adds a native `ControlSnapshot` over one complete projectless catalog
observation and a selected history index. It retains the exact publication and
candidate proof for live stop/rename, awaits process exit before stop success,
and uses actual pipe-peer authentication for explicit incompatible force.
Read-only ready observations do not authorize cleanup. A stopped-name edit stays
owned through pending recovery; a verified rename remains successful if its
history cache refresh fails. Clean forgets only proven unchanged stopped rows.
The sequential stop-all owner retains admitted unknown outcomes across caller
cancellation, attempts the remaining distinct hosts, and bounds detail bytes
while keeping complete counts.

Independent review found no remaining findings after the pending-recovery owner,
reporting and fixture corrections. Five focused action tests pass. Formatting,
all-target Clippy with warnings denied and the full native workspace suite pass:
3,293 tests, zero failures, 58 ignored across 46 libtest/doc-test groups, plus
six native LSP transport cases. Native CI acceptance remains pending.

## Accepted package: 4e.4b native selector-only CLI

`a24b943` dispatches list/rename/explicit selected stop/stop-all/clean before
resolving a current project. It captures roots once, uses `DiscoveryScope`
without an invented cwd project, and resolves selected rows once against a
complete snapshot. Shared pure table presentation preserves the Unix columns
and order. Windows stop without an explicit selector and restart remain
unavailable; public attachment remains gated. README, user guide and CLI help
describe the accepted native controls and their limits.

Six real-host CLI acceptance tests pass: nonproject cwd, live and stopped
rename/clean, normal and forced exit, protected stop-all refusal with another
host exiting, incomplete exact ready observation refusing list/clean without
history mutation, and same-project publications in isolated namespaces.
Independent review found no remaining findings. Formatting, all-target Clippy
with warnings denied and the complete native workspace suite pass: 3,299
tests, zero failures, 58 ignored across 47 libtest/doc-test groups, plus six
native LSP transport cases. Native CI acceptance remains pending.

## Accepted package: 4e.5a typed row identity

`b5c47ce` adds `WorkspaceSelection`: an exact native live publication key
beside its project, or project-only identity for Unix and stopped rows. The
fixed native key digests framed exact project bytes, process identity,
incarnation and pipe address after peer authentication; display names and
health do not change it. Complete snapshots reject duplicate selection keys,
and selected lookup refuses a stale replacement without falling back to path,
name or PID. Independent review found no remaining findings. Focused shared
value and native catalog/history tests pass. Formatting, all-target Clippy with
warnings denied and the full native workspace suite pass: 3,301 tests, zero
failures, 58 ignored across 47 libtest/doc-test groups, plus six native LSP
transport cases. Native CI acceptance remains pending.

## Accepted package: 4e.5b manager selection propagation

`6befd91` carries `WorkspaceSelection` through manager refresh, preview and
inventory requests, cache and completion, action menus, force confirmation,
delayed prompts and selected action completions. A refresh cannot redirect a
captured action to another same-project publication. Unix service requests
reject native-key selections before path resolution; colon and worktree path
APIs remain separate. Native manager availability and attachment remain gated.
Independent review found no remaining findings. Focused manager and worker
regressions pass, as do formatting, all-target Clippy with warnings denied and
the full native workspace suite: 3,301 tests, zero failures, 58 ignored across
47 libtest/doc-test groups, plus six native LSP transport cases. Unix-only
regressions still need CI after the push; no native CI claim is made.

## Accepted package: 4e.5c owned native catalog service

`92e44dc` adds one native catalog worker thread with a current-thread Tokio
runtime, bounded request/event queues, a latest-preview slot, and joined
shutdown. Standalone startup captures a discovery scope after resolving its
actual project; the native host reuses its verified layout and current ready
location. The worker retains exact selected publication proof and the pending
stopped-name recovery ledger across refresh failure and shutdown. User-authored
selectors use a fresh complete observation. Read-only discovery and preview
cancel on shutdown; begun mutations stay worker-owned. Missing Git returns an
empty worktree result without a failed spawn loop. The service handle and events
are retained by `HostServices` while the native manager gate remains closed.

Independent Astra review found no remaining findings after the stop and event
backpressure, admission, and cleanup corrections. Nine focused native service
tests pass, including a real peer preview, publication replacement, held-peer
shutdown cancellation, pending stopped-name recovery, and full event queue
shutdown. Formatting, all-target Clippy with warnings denied and the complete
native workspace suite pass: 3,310 tests, zero failures, 58 ignored across 47
libtest/doc-test groups, plus six native LSP transport cases. Native CI and Unix
coverage acceptance remain pending a later authorized push.

## Accepted package: 4e.5d native manager controls

`5b86575` gives Windows standalone editors a separate `SessionControls`
capability while attachment still requires the unavailable persistent frontend.
The existing manager lists and previews native rows, captures exact selected
publication keys for rename, normal stop and force stop, and handles service
events directly. `:session-clean` performs global verified stopped-history
cleanup; colon stop/rename use explicit user selectors and do not borrow a
highlighted row. The manager does not claim a current Windows publication,
assign digits or offer attachment, inventory, cycling or destination visits.
It requires explicit reselection after a highlighted publication disappears.
README, user guide, keymap register, UI vocabulary, command availability, help
and hints state those limits.

Independent Astra review found no remaining findings after availability,
preview, menu, completion and stale-selection corrections. Five focused native
UI tests and two real-host manager acceptance tests pass. The latter exercise
two publications of one project, exact preview and rename, protected stop
refusal, confirmed force exit with the unrelated host alive, and a captured
rename prompt refusing a new publication at the same location. Formatting,
all-target Clippy with warnings denied and the complete native workspace suite
pass: 3,318 tests, zero failures, 58 ignored across 47 libtest/doc-test groups,
plus six native LSP transport cases. Native CI and Unix coverage acceptance
remain pending a later authorized push.

## Accepted package: 4e.5e native process-exit supervision

`1359c1a` adds a one-shot native wait over the retained host process handle and
uses it for stop completion under the existing five-second budget. It no longer
polls the process during an active stop. The callback keeps its context and
process handle through completed unregistration; an unexpected unregister
failure retains them. One notifier task isolates caller wakers from the native
callback. A sticky exit flag preserves cancellation and repeated waits.

Independent Astra review found no remaining findings. Focused tests cover
already-exited processes, cancellation, callback/drop overlap, runtime shutdown,
registration and unregister failures, and reentrant caller-waker destruction in
compiled helpers with parent timeouts. Formatting, all-target Clippy with
warnings denied, and the complete native workspace suite pass: 3,325 tests,
zero failures, 61 ignored across 47 libtest/doc-test groups, plus six native LSP
transport cases. Native CI and Unix coverage acceptance remain pending a later
authorized push.

## Accepted package: 4e.5f native foreground parent identity

`4aa792b` obtains a candidate parent PID from one bounded ToolHelp snapshot,
then immediately opens a retained same-account process handle and rejects a
candidate created after the child. A missing parent, an exited parent before
pinning, and a detached host's synthetic inheritance parent have explicit
outcomes. Only an `OpenProcess` absence is classified as gone; later identity,
owner and liveness query errors remain errors. This value does not yet supervise
a foreground host or authorize terminal requests.

Independent Astra review found no remaining findings after error classification,
atomic fixture report publication and process-tree cleanup corrections. Compiled
helpers prove live parent identity, rejection of a later-created process, and
the original retained parent handle after its exit. Formatting, all-target
Clippy with warnings denied, and the complete native workspace suite pass:
3,330 tests, zero failures, 62 ignored across 47 libtest/doc-test groups, plus
six native LSP transport cases. CI run 35745358533 exposed a Unix test compile
error: the inventory identity test accessed private navigation fields. `4a3b1bd`
replaces that access with an immutable test-only accessor and preserves the
test's generation and selected-publication assertions. Its cross-platform CI
result is pending the next push; the run above must not be called passing.

## Accepted package: 4e.5g typed native console termination

`c6dea6a` registers one Windows listener owner before launch parsing and keeps
it through standalone and detached-host cleanup and logging. Ctrl+C, Ctrl+Break
and console Close remain distinct typed events. The native host returns through
its existing joined service and transport cleanup; standalone input and draw
errors also reach joined cleanup. A Close arriving during cleanup or before
startup returns is reconciled before any top-level error reporting to a dead
console. The user guide states the operating system's limited Close deadline.

Independent Astra review found no remaining findings after cleanup and fixture
ownership corrections. Compiled helpers use an isolated new console and prove
real Ctrl+C and Ctrl+Break delivery without signaling the test runner. Close
routing tests cover startup, input and cleanup failures and Close after Ctrl+C.
Formatting, all-target Clippy with warnings denied, and the complete native
workspace suite pass: 3,333 tests, zero failures, 63 ignored across 47
libtest/doc-test groups, plus six native LSP transport cases. Native CI and Unix
coverage acceptance remain pending the next push.

## Accepted package: 4e.5h native foreground host supervision

`ab9f3ac` captures and pins the natural parent of foreground `--serve` before
configuration startup, then retains one process-exit watcher through joined
host cleanup. Parent loss before publication refuses the launch; parent loss
while serving retires only that host. Typed console termination remains distinct.
Detached hosts never observe their temporary inheritance parent. Foreground
project resolution accepts an explicit `--project-root` or discovers an existing
project; if neither exists it refuses with an actionable path instead of entering
a blocking prompt that could outlive the parent. `--init` is already refused with
`--serve` by argument validation.

Independent Astra review found no remaining findings after removing the blocking
prompt, restoring fair host service selection, and tightening real-process
fixtures. The focused native host suite passes 9 active cases. The fixtures
exercise parent exit before publication and while serving, exact ready-record
retirement, unrelated host survival, detached launcher loss, project discovery
and refusal, and owned process-tree cleanup. Formatting, all-target Clippy with
warnings denied, and the complete native workspace suite pass: 3,338 tests,
zero failures, 64 ignored across 47 libtest/doc-test groups, plus six native LSP
transport cases. Native CI and Unix coverage acceptance for this commit remain
pending its push.

The preceding `219acba` CI run found a Unix `clippy::let_unit_value` error in two
never-completing catalog branches. `6bd7627` keeps their Unix placeholder result
non-unit; its CI run 35749011529 has passing Ubuntu gates, MSRV and both coverage
jobs. MacOS ru-time temporary cleanup and Ubuntu lifecycle stress failed in
unrelated test paths; investigate repeatability rather than calling that run
green. Its native Windows job was still running at this checkpoint.

## Accepted package: 4e.5i internal native host attachment

`1e69755` admits one authenticated interactive pipe peer in the native host.
The retained peer proof and connection ID own input, geometry, hints, full-frame
publication and subscribed waits. Controls cannot send physical input or invoke
editor commands. A second interactive peer is refused; a stale old connection ID
cannot redirect input or disconnect a replacement. Explicit disconnect cancels
only that peer's pending waits. Editor `:detach` and `:quit` complete subscribed
waits before the terminal response; `:quit` still refuses while another control
client has protected state. Native directory handoff remains disabled until a
real frontend can deliver the selected directory to its shell.

`5d727db` repairs shared response receiving so a final `ShuttingDown` cannot
overtake an already queued semantic command result when both lanes become ready
during one `select!` poll. Its regression forces that interleaving.

Independent Astra source review found no remaining findings. Real native pipe
fixtures cover Welcome and initial full frame, a unique attachment, control
input refusal, editing and resize, wait ownership across disconnect/reattach,
disabled `:quit-here`, ordered command/wait/shutdown replies, actual host exit,
and stale connection IDs. Formatting, all-target Clippy with warnings denied,
and the complete native workspace suite pass: 3,342 tests, zero failures and
64 ignored entries across 47 libtest/doc-test groups. The focused response
ordering regression and 11 active real-host integration cases also pass.
Public attachment, switching, manager visit, numbered sessions, restart and
parent-terminal routing remain gated.

CI run 35751559497 for preceding `63f8ea6` passed Ubuntu gates, MSRV, both
Unix coverage jobs, macOS tests and all other jobs except Native Windows. That
job repeated an intermittent 15-second Windows PowerShell filter timeout in
`pipe::windows::tests::failures_bounds_and_whole_job_budget`. The child started
promptly but remained alive; two recent runs stalled at different best-effort
trace markers. No safe Runyte execution fix is established. Retain the existing
deadline and diagnostics pending an isolated reproduction or process dump.
CI run 35755224602 for `70f8c69` passed the native host integration cases and
Unix coverage gates. Native Windows failed two recurring PowerShell filter
timeouts: both children spawned in six milliseconds and remained alive without
output or exit through the 15-second budget. The trace markers were `missing`
and `policy-set`; neither establishes the cause. Ubuntu plugin conformance
failed because the fixture could match its own echoed terminal command before
the child produced output. macOS plugin conformance exposed `EINTR` from the
nonblocking host supervisor process queue read. The two targeted repairs above
are committed locally and require CI validation after their push.

## Accepted package: 4e.5j1 private native frontend attachment

`180175d` adds one internal Windows TUI attachment owner over the authenticated
buffered pipe. It retains the actual connected process handle, races writes and
input against that exact peer's exit, preserves semantic error replies during
the bounded final drain, and restores terminal mode on every return. Welcome
and the first complete frame share one startup deadline. Empty native Paste
reaches the host as ClipboardPaste, where the editor decides its meaning.
Public attachment and switching routes remain gated.

The ConPTY acceptance fixture runs a real native host outside the frontend's
ConPTY. It covers editing and save, a newly rendered resize frame, busy refusal,
detach and reattach, host death, Ctrl+Break cleanup, and a connected server
that withholds its first frame. Independent Astra review found no remaining
findings after exit-aware writes, reply ordering and resize-fixture corrections.
`tests/input_boundary.rs` allows Crossterm only in the TUI adapter source.
Formatting, all-target Clippy with warnings denied and the complete native
workspace suite pass: 3,345 tests, zero failures and 72 ignored entries across
47 libtest/doc-test groups, plus six native LSP transport cases.

`dad1d86` retries only interrupted macOS `kevent` process-queue observations
against the retained queue. An injected macOS regression observes EINTR then
NOTE_EXIT. Independent Astra review found no blockers. The subsequent macOS
plugin job reached the later Python fixture assertion, validating that host
repair. `26f6c4e`, `333771d` and `0adcf38` make the Unix plugin fixture wait for
short child-emitted terminal output and compact rendered rename status within
one deadline. Two CI runs exposed the command-echo and rendered-space races;
the final adjacent-quoted shell command is independently reviewed but still
needs Unix CI execution. Python is unavailable on this Windows host.

## Accepted package: 4e.5j2 typed switch intent

`8657281` replaces selector plus previous-session flags with explicit
`WorkspaceSwitchTarget` variants for user selectors, captured
`WorkspaceSelection` values and previous-session intent. Manager selection,
numbered and cyclic navigation, destination visits and prepared session-strip
targets retain publication keys. Colon, explorer, terminal-directory and Git
worktree paths remain authored selectors. Private protocol version 55 carries
the target with strict nested decoding and a fixed 32-byte optional publication
key; Unix rejects a keyed native target before path resolution. Public Windows
attachment and navigation gates remain closed.

Independent Astra review found no remaining blockers after strict unknown-field
rejection, the boxed wire payload, the Unix compile correction and the restored
internal platform check. Same-project strip and cycle activation still require
the current native attachment identity and belong to the private switching
loop. The strict wire regression passes locally. The Unix same-project
propagation and keyed-refusal cases are compiled only on Unix and require CI.

## Accepted package: 4e.5j2b exact native target preparation

`ec33c26` adds a bounded preparation request to the owned native catalog worker.
It observes again through the worker's frozen scope, current known location and
hidden-publication choice, selects the complete captured `WorkspaceSelection`,
and returns exact endpoint metadata with its retained authenticated
`PinnedProcess`. Project-only, stale, replaced, incompatible and incomplete
targets refuse without path, name or PID fallback and without host creation.
Renaming the same publication preserves its key.

Preparation has one three-second admission-to-result deadline, observes service
shutdown, skips abandoned queued requests and cancels an active health probe
when its caller leaves. Independent Astra review found no remaining blockers
after the backpressure and cancellation regressions retained real response
ownership and exercised in-flight shutdown. All 15 native service tests pass.
Formatting, all-target Clippy with warnings denied and the full Windows workspace
suite pass across the combined j2 packages: 3,351 tests, zero failures and 72
ignored entries across 47 libtest/doc-test groups, plus six native LSP transport
cases. Unix compilation and coverage require CI after push.

## Accepted package: 4e.5j3 private exact switching loop

`11963b1` implements a two-phase private native switch. The source connection
and its interactive reservation remain owned while the source host prepares an
exact selected publication through its catalog service. The frontend
independently authenticates the destination and draws its first complete frame
before sending a receipt-matched commit. Abort resumes the retained source
without discovery or reconnection. A lost commit acknowledgement is treated as
uncertain: the frontend closes the destination and exits within the whole
operation deadline without replaying the commit or assuming success.

Same-project publications compare their complete `WorkspaceSelection` keys.
An exact-current selection is a no-op. Commit cancels only source-owned waits;
abort preserves them and control-client waits continue through a pending
switch. Source-owner loss cancels active preparation immediately. Commit and
abort requests are interactive-only, and public attachment, manager visits,
numbered sessions, restart and parent-terminal routes remain gated.

Independent Astra review found no remaining blockers after request-role,
asynchronous reply, preparation cancellation and fixture corrections. The real
ConPTY acceptance uses three native hosts and proves busy-destination abort with
continued source editing, A-to-B-to-A switching with saved edits, an
exact-current no-op followed by a fresh edit, and lost-acknowledgement bounded
exit followed by successful destination reattachment. The pre-existing native
frontend acceptance also passes.

## Accepted package: 5a native plugin worker foundation

`ce888ee` adds the native Windows plugin process worker while leaving public
plugin startup unavailable. Shared NDJSON supervision is transport-generic;
the Windows adapter uses overlapped private stdin/stdout, a null stderr handle,
the existing isolated native process launcher and one retained launch-handle
exit watcher. Complete frames already read from stdout are delivered before the
terminal `WorkerStopped` event, including an exit or write-error race. Partial,
malformed and oversized frames fail closed, and an uncertain partial write
suppresses a registration rejection frame.

Cleanup terminates the private job and reports `reaped` only after the exact
leader handle is signalled and job accounting reports no active processes.
That contract proves the direct child is settled and no descendant is running;
an externally retained historical descendant process object may become
signalled immediately afterward. The acceptance fixture waits that handle with
a bounded kernel wait before deleting its storage.

Independent Astra review found no remaining blockers. The harness-free native
acceptance passes seven cases: fragmented/coalesced framing and exit order,
saturated output with the reserved final event, malformed/oversized input,
write-failure final-reply drain, blocked-write cancellation, descendant
settlement and runtime-shutdown ownership. Formatting and all-target Clippy
with warnings denied pass. The full Windows workspace suite passes 3,369 tests,
zero failures and 78 ignored entries across 47 libtest/doc-test groups,
including the seven native worker cases, plus six native LSP transport cases.
CI for these two new commits requires their push. CI run 35766489261 for the
preceding `1935ff9` checkpoint is fully green on Windows, Linux and macOS,
including both coverage gates and plugin conformance.

## Accepted package: 5b durable Windows plugin state

`140347e` adds optional `workspace.state_anchor: profile | local-app-data` and
resolves it only through Windows known folders. `workspace.state` remains the
sole destination and must be a proper descendant of that anchor. The workspace
state owner captures the policy once. Arbitrary or inferred fallback anchors,
relocation and migration are absent. Unix parses the portable setting while
retaining its existing storage behavior. Omitting the anchor preserves the
existing full-ancestry storage policy and its refusals.

The Windows storage path traverses existing ordinary parents through retained
handles without creating or hardening them, flushes the required ancestors,
and rejects reparses, unsupported volumes and reserved-storage overlap under
native prefix and case rules. It privately creates or admits only the final
state root; existing descriptor-relative children then provide plugin state.
Native get, set and delete retain canonical revisions, cross-process locks,
private pending-file recovery, cancellation before mutation and
`outcome_unknown` after promotion. Public plugin startup remains unavailable.

Independent Astra review accepted the package after native alias/overlap,
ordinary-ancestor ACL and linked/nonprivate-file corrections. The durable-state
filter passes five active cases with one compiled process helper ignored,
including cross-process locking and post-promotion uncertainty.

## Accepted package: 4e.5j4 private native parent waits

`fa18a52` adds private Windows ParentWait without opening ParentAttach or public
`--wait`. A BCrypt capability marker carries exact endpoint metadata through
the explicitly launched PTY environment. Admission requires the retained pipe
peer to belong to the exact terminal job, the terminal capability to match, the
origin terminal to be visible, and the active attachment generation to remain
current. Interactive readiness acknowledges a frame issued for that generation;
it indicates a drawn frontend and never grants approval.

The internal exact-endpoint client bounds connect, Hello and ParentWait under
one deadline and races every stage and write against natural-parent loss.
Cancellation starts its own deadline before `CancelWait`, uses bounded recovery
on the same reader and never retries a poisoned writer. A lost natural parent
cancels only that client's wait; the shared host, frontend and unrelated waits
remain live. Completed ownership is retained until host history prunes the
token.

Independent Astra review accepted the corrected authority, deadline, readiness,
retention and ConPTY fixture contracts. The ParentWait filter passes three
active cases with four compiled helpers ignored. The exact parent-context and
terminal-job filters each pass; the full ConPTY authority and natural-parent-loss
acceptance passes; the response-backpressure regression passes ten consecutive
runs.

Final direct validation for the combined accepted checkpoint is green:
`cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, and
`cargo test --locked --workspace --no-fail-fast` all exit zero. The saved log's
47 summaries contain 3,382 standard passing cases and 85 ignored entries. With
the seven custom native plugin-worker cases, the established total is 3,389
passed and 85 ignored across 47 groups, plus six native LSP transport cases.

The Unix import correction in `c4f8d18` is fully accepted by CI run
[35786843607](https://github.com/runyte/runyte/actions/runs/35786843607), including
Windows, Linux and macOS jobs and both coverage gates.

## Accepted package: 4e.5j5 private native ParentAttach

`42f2c9c` adds private Windows ParentAttach without opening public attachment,
`--wait`, manager visits, numbered navigation, restart or directory handoff.
Admission reuses the ParentWait authority: the retained requesting pipe peer
must belong to the exact ConPTY job, its terminal capability must match, and the
current ready attachment generation must still be owned by the retained source
frontend. An active origin terminal and parent-request readiness are required;
marker text, metadata PID and ancestry do not authorize the request.

The parent host's worker owns destination preparation. It uses frozen discovery,
configuration, state and executable inputs, observes a fresh complete catalog,
and either retains the exact live destination proof or initializes and starts
the exact directory destination outside the requesting terminal's job. Caller
cancellation does not abandon admitted work: the worker retains provisional
startup cleanup through settlement and resolves the start-versus-existing-host
winner before returning one atomic commit disposition.

Commit remains provisional until the original retained source frontend receives
and confirms the source host's nonfinal commit acknowledgement. The service
supplies a still-armed destination decision. The source host revalidates the
retained child and frontend proofs, captured attachment generation, terminal
capability and deadline before committing, then retains the result receiver
through worker settlement. Successful settlement sends `ParentAttached` to the
child and releases the source reservation with the frontend’s final committed
receipt. Expected source-connection closure after confirmation is permitted; it
is not a prerequisite for child success. The same exact source is a no-op, and
ordinary native switching keeps its existing acknowledgement behavior.

Independent Astra review accepted the authority, fresh resolution, startup and
cleanup ownership, source-frontend confirmation, settlement revalidation,
same-source handling, private routing and real ConPTY acceptance. Focused
validation passes six service lifecycle tests, two host-client tests and the
real ConPTY acceptance. Final validation is green: `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings`, and
`cargo test --locked --workspace --no-fail-fast` all exit zero. Core summaries
report 2,755 lib tests passed with 24 ignored and 66 bin tests passed with 29
ignored; every integration target is green, along with seven custom native
plugin-worker cases and six native LSP transport cases.

CI run [35799397233](https://github.com/runyte/runyte/actions/runs/35799397233)
for the combined `777dbf3` ParentAttach checkpoint is fully green on Windows,
Linux and macOS, including both coverage gates, plugin conformance, lifecycle
stress, MSRV and the release floor.

## Accepted package: 5c private Windows plugin handoffs

`8148437` adds private Windows `terminal.open` and `external.open` for an already
registered native plugin while leaving public plugin discovery, startup, stop and
restart gated. Registered commands remain executable because their worker and
capabilities already exist; no Windows lifecycle command can start a worker.
Native handoff authority is separate from ordinary plugin foreground authority.
Only a current, unmodified, nonreplayed physical Enter on the attached frontend
can grant it, and validated forms retain that grant only with the exact queued
and in-flight revision and foreground generation. Repeats, later input,
attachment replacement, plugin restart, stale validation and replay consume or
invalidate it.

Unpublished terminals have eight slots and four MiB of accounting each. Windows
starts the ConPTY child suspended, retains setup and cleanup ownership across
every partial failure, and releases the lease only after the exact child, job,
reader, writer and lifecycle work settle. Installation resumes the child and
transfers ownership to the ordinary terminal collection, so the installed
terminal survives plugin stop. Cancellation drains and discards unpublished
output off the editor loop. Captured working directories require the exact
workspace identity and reject parent escape and junction traversal.

External open reuses the native prepared-launch dispatcher and revalidates the
captured invocation before irreversible admission. A lost or cancelled result
after that boundary is `outcome_unknown` and cannot be replayed as a safe
refusal. Source review also accepted attachment and plugin-generation rejection,
literal arguments, one-shot authority and public-gate preservation.

Independent Astra review found and closed the partial-setup child wait and
asynchronous validated-form authority gaps; the final review has no findings.
Focused coverage passes all four native pending-terminal tests, four host handoff
tests, the four partial ConPTY setup checkpoints, eight simultaneous cleanup
owners, repeat/form authority cases, install transfer, cwd containment, stale
attachment/plugin results and external launch. Final validation is green:
`cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, and
`cargo test --locked --workspace --no-fail-fast` all exit zero. Core summaries
report 2,765 lib tests passed with 24 ignored and 66 bin tests passed with 29
ignored; every integration target is green, along with seven custom native
plugin-worker cases and six native LSP transport cases. Native CI acceptance for
this commit remains pending.

## Next packages: context access and the Windows bridge

Continue context transport and grants and the Windows Python MCP bridge.
Context identity and grant storage uses its separate LocalAppData policy and
must not inherit the plugin state anchor. Public persistent attachment and wait
routes, manager visits, numbered sessions, restart, directory handoff and
combined Git removal remain closed until their own acceptance gates pass.

## Phase 2.5 implementation order

The [catalog/service design](../plans/active/WINDOWS_CATALOG_SERVICE.md) records
the reviewed integration map and acceptance requirements. DiscoveryScope, which
that design calls a prerequisite, is already applied as described above.

1. Completed in 4e.4a and 4e.4b: native control orchestration and CLI for
   list/rename/stop/stop-all/clean
   against exact retained publications. Stop success requires actual process
   exit, not an acknowledgment or missing ready file. Aggregate stop-all failures
   while attempting other distinct hosts. Keep restart separately gated until
   real-host stop-to-start acceptance. Extract pure CLI table presentation while
   preserving Unix behavior; selector-only commands must not invent a project.
2. Shared typed row selection and an owned native catalog service. Duplicate
   same-project publications require separate selection, preview, prompt and
   completion identities. A stale publication key must never fall back to a path,
   name or PID. Keep native proof in the service, not display DTOs. Use one owned
   runtime/thread, bounded admission/events and joined shutdown; App holds only
   a sending handle. Missing Git remains supported during worktree discovery.
3. Accepted foundations: process-exit supervision, private native frontend
   attachment, exact switching, ParentWait and ParentAttach. Preserve common
   editor semantics, protected shutdown and connection-owned waits while opening
   only the public availability gates authorized by the plan.
4. Completed in 4e.5j5: ParentAttach authorization and routing using the
   [reviewed parent design](../plans/active/WINDOWS_PARENT_ROUTING.md). The
   retained pipe peer's exact ConPTY job membership, terminal capability and
   current attachment ownership agree through settlement. Destination startup
   remains outside the requesting terminal job, and ConPTY job limits are not
   relaxed for detached startup.
5. Acceptance and documentation, including combined Git branch/worktree removal
   through the now-native persistent coordinator.

Detached and foreground native hosts now have real process tests. The host owns
one internal physical-input attachment and private ParentWait and ParentAttach
are accepted, while public attachment and wait routes remain gated pending their
authorized acceptance paths.

### Reviewed process-exit watcher contract

Use retained process handles and a one-shot RegisterWaitForSingleObject wait,
not a timer in the idle editor. Capture the current Tokio runtime handle when
constructing the watcher. Retain stable callback storage and the process handle
until completed UnregisterWaitEx with INVALID_HANDLE_VALUE. A failed unregister
must not free memory still reachable by a callback.

The callback sets a sticky exit flag with release ordering, then schedules one
notification task holding only shared signal state. It must not invoke arbitrary
caller wakers directly: a waker could drop the watcher reentrantly and deadlock
unregistering its own callback. Waiters enable notification before acquiring the
sticky flag. Cancellation and repeated waits must preserve observed exit.
Retain the bundle rather than risk use-after-free if unregister unexpectedly
fails. A captured Tokio handle alone does not keep its runtime alive.

The watcher implementation pre-spawns its single notifier task on the captured
runtime at registration. The callback releases the sticky flag and sends a
one-shot signal that schedules that task; it never calls the runtime after
registration or wakes a caller directly. This also permits a runtime to shut
down before the callback without invoking a dead runtime handle. Active wait
delivery still requires that captured runtime to run; shutdown cancels its
notifier task. Completed unregister remains the sole condition for freeing the
retained process and callback bundle.

Acceptance includes already-exited processes, cancelled/repeated waits, callback
and drop races, and reentrant caller-waker destruction in an owned compiled helper
with a parent timeout. Parent discovery must immediately pin a candidate handle,
verify the same user and creation order, and reject a parent created after its
child. This is supervision, not terminal authorization. Native Ctrl+C, Ctrl+Break
and console-close events are distinct; subscribe across startup and document
console-close cleanup as limited by the operating system deadline. Detached hosts
survive authenticated launcher handoff; losing a wait client's parent cancels its
wait without killing an unrelated shared host.

## Remaining 2.6

Native plugin worker/process ownership and framing is accepted in `ce888ee`, and
durable plugin state is accepted in `140347e`. Private Windows `terminal.open`
and `external.open` handoffs and physical-frontend approval are accepted in
`8148437`. Public plugin startup remains gated.

Implement context transport/grants and the Windows Python MCP bridge next. Keep
context identity and grants on their separate LocalAppData policy. Validate
immutable and current Node/plugin conformance plus real Windows context clients.

The Node reader buffered-publication fix and bounded diagnostics are committed
in `aadaf48`. The original intermittent initial-registration failure's cause is
still unproven; `context/issues/node_conformance_readiness.md` stays open despite
recent green CI. Deferred macOS PTY allocation work is outside this task.

## Execution constraints and handoff safety

Serialize native builds and tests. Set these in each new PowerShell process:

```powershell
$env:CARGO_BUILD_JOBS='1'
$env:RUST_TEST_THREADS='2'
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --workspace --no-fail-fast
```

Check each exit code before advancing. Agents review/stage without concurrent
Cargo or native probes; the coordinating agent owns native execution. About
17.5 GiB usable RAM was available. The earlier hard reboot has no established
diagnosis; do not claim memory exhaustion caused it. Never terminate unrelated
processes to recover capacity.

Use GitHub CLI for authenticated CI logs; if PATH has not refreshed, its installed
location is `C:/Program Files/GitHub CLI/gh.exe`. A completed job's log can be read
with `gh api repos/runyte/runyte/actions/jobs/JOB/logs --allow-escape-sequences`
while the overall run is still active. Do not claim a queued/running job passed.

Every editor/host fixture must own temporary XDG_CONFIG_HOME and relevant
configuration/cache/state. Use compiled or checked-in executable fixtures;
never execute a program written by a test. Retain process/job ownership through
cleanup before deleting fixture storage. Native process checks may require the
normal token outside the sandbox, as in preceding acceptance runs.

The coordinating agent has no pending validation command, and the naming
author/reviewer have finished. A separate `cargo run --release` was observed
during the handoff; its ownership is outside this validation work. Leave it alone
and check for overlapping builds before starting new validation. Closing the chat
does not require a live agent to finish a transaction. The discovery-scope source
is included in this checkpoint. Ignored naming-package staging remains local to
this checkout; a different checkout will need that package transferred or
reconstructed from the retained contract. Inspect git status before editing.
