# Windows Phase 2 continuation

Checkpoint: 2026-09-23, branch `feat/windows-support`, private native
ParentAttach is accepted in `42f2c9c`, durable Windows plugin state is accepted
in `140347e`, private native parent waits are accepted in `fa18a52`, and private
Windows plugin handoffs are accepted in `8148437`. Native Windows context
identity/grant storage and repeat-safe context approval are accepted in
`61acb18`, and private Windows context transport and discovery are accepted in
`439f4da`. Private Windows context host grants are accepted in `bbdb12a`, and
the private Windows Python MCP bridge is accepted in `34f53b6`. Private exact
native switching is `11963b1` and the native plugin worker foundation is
`ce888ee`. Typed switch intent and exact native target preparation are
`8657281` and `ec33c26`. Private native frontend attachment is `180175d`.
Public Windows plugin discovery, startup, stop and restart are accepted in source
commit `9743bd9`. Explicit public Windows persistent attachment is accepted in
source commit `2563fba`; configured bare Windows attachment is accepted in
source commit `378a22b`. Guarded CLI-only Windows session restart is implemented
in source commit `8a8987c`, with detached-capable replacement acceptance pending.
Native Windows `Ctrl-\` decoding and review acceptance are `eff117c` and
`1159418`, with resolved issue record `e34d04c`. Public integrated-parent
Windows `--wait` is accepted locally in `fee4819`.
Selected live-row visits from the Windows persistent session manager are
accepted locally in `92329b0`.
Public Windows context access is accepted in source
commit `a79e765`; Windows CI acceptance regressions are repaired in source commit
`05935a6` and remotely accepted through handoff commit `d693832`.
The macOS host queue EINTR repair is `dad1d86`; Unix plugin fixture readiness
repairs are `26f6c4e`, `333771d` and `0adcf38`. The preceding native host
attachment is `1e69755` and shared response ordering repair is `5d727db`.
This record supplements the [active plan](../plans/active/PLAN_WINDOWS_PHASE2.md)
with the working-tree state and immediate continuation steps. Read this record
before the older chronological progress entries. No previous chat is required.

## Scope and delivery

Phase 1 is complete. Phase 2.1 through 2.4 and the native Ctrl+h/Ctrl+j
correction are complete. Phase 2.5 has accepted private ParentWait and
ParentAttach paths, explicit public `-a`/`--persistent` attachment and configured
bare attachment. Guarded CLI restart has local refusal acceptance; successful
stop-to-start acceptance remains gated. Public `--wait` from an authenticated
persistent integrated terminal is accepted; persistent wait from an ordinary
shell, numbered navigation, directory handoff and other in-editor switching
remain gated. The persistent manager can visit a selected compatible running
publication; the standalone manager remains control-only.
Phase 2.6 has accepted native worker, durable-state, private handoff,
context-grant storage, context transport/discovery, host grants, the Python MCP
bridge, public Windows context access and the public plugin lifecycle. The
managed plugin helper-process capability remains unavailable on Windows.
Integrated Git is optional: missing
Git must leave the integration disabled without failed spawn loops or runtime
errors. Combined branch/worktree deletion still needs the native
persistent-session coordinator; separate guarded operations work.

Continue sequential work packages with an independent subagent review after
each package. Incorporate findings and repeat review until none remain before
advancing. Commit accepted checkpoints locally. Earlier authorization covered
pushes to `feat/windows-support`, but automatic approval review denied the
latest push; do not retry it without explicit approval. The exact push through `d9361e6` to
`git@github.com:runyte/runyte.git` succeeded, and the later push through
`777dbf3` succeeded. The exact pushes through public-context handoff commit
`46311e7` and remediation handoff commit `d693832` also succeeded. The remote
branch therefore includes `a79e765`, `05935a6`, `d693832`, `9743bd9`,
`2563fba`, `378a22b` and `8a8987c`. The plugin source checkpoint passed every job in
[CI run 35875460337](https://github.com/runyte/runyte/actions/runs/35875460337).
Direct attachment source commit `2563fba` has local acceptance; its
[CI run 35881502895](https://github.com/runyte/runyte/actions/runs/35881502895)
was cancelled by the next push. Configured bare attachment source commit
`378a22b` passed all jobs in
[CI run 35883007224](https://github.com/runyte/runyte/actions/runs/35883007224).
Restart source commit `8a8987c` passed every job in
[CI run 35888094832](https://github.com/runyte/runyte/actions/runs/35888094832),
while successful stop-to-start acceptance still needs a detached-capable
Windows runner. The `Ctrl-\`, public ParentWait and manager-visit commits remain
local; automatic approval review denied the push, and they have no remote CI
yet.
Report completion of the entire Phase 2 or an unexpected blocker requiring a
decision. Keep ordinary-shell persistent wait, numbered navigation and other
in-editor switching behind their own acceptance gates.

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

## Accepted package: 5d private Windows context transport

`439f4da` adds private Windows context registration, exact publication ownership,
named-pipe transport and metadata-only discovery while leaving context host
startup, grants and public `--context-list` unavailable. Discovery v1 keeps its
string path fields. Windows registrations add process creation time and reject a
non-Unicode workspace root before publication. Endpoints use only the strict
`\\.\pipe\runyte-context-v1-<64 hex>` family.

The pipe uses a private ACL, rejects remote clients, pins the actual same-account
peer process and limits ownership to eight active or closing streams plus one
pending instance. Frames are newline-delimited and bounded to two MiB. Shutdown
and lease revocation cancel queue admission, reply waits and writes. Every owner
exit signals cancellation, joins peer tasks and retires only its exact
registration; cancelled shutdown retains the join handle for a later await.
Discovery opens only an existing private store, probes at most 64 endpoints with
eight concurrent 250 ms probes and a three-second overall deadline, and accepts
only the exact registration returned by the authenticated live peer.

Independent Astra review found no remaining findings after connection permits,
close-event pressure, joined error cleanup, cancellable shutdown and native test
coverage were corrected. Ten real named-pipe transport tests and three native
discovery tests pass. Final validation is green: `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings`, and
`cargo test --locked --workspace --no-fail-fast` all exit zero. Core summaries
report 2,788 lib tests passed with 24 ignored and 66 bin tests passed with 29
ignored; every integration target is green, including seven native plugin-worker
and six native LSP transport cases. Native CI acceptance remains pending.

## Accepted package: 5e private Windows context host grants

`bbdb12a` privately enables the shared context host semantics, reads and frame
capture on Windows while normal context startup, `:context-access` and public
`--context-list` dispatch remain gated. Startup captures one `StorageLocation`
and environment fingerprint. Remembered grants are staged atomically through a
read-only existing store before that location is reopened for writes. Native
registration binds the current process PID and creation time, and the transport
owns the exact registration publication through retirement.

The host retains at most one active or retiring server. Its 33-slot event
channel permanently reserves one lifecycle permit, preserving the existing 32
ordinary event slots. Last revoke immediately cancels semantic authority and
starts joined retirement. The completion-driven `Event::Retired` path performs
at most one deferred re-enable after successful retirement. A retirement or
bind failure latches the listener closed until a fresh physical Grant decision
explicitly retries it. Frame admission rechecks active authority after UI grant
or revoke synchronization, and cancellation-safe context shutdown is aggregated
before plugin teardown in standalone and native-host cleanup.

Real acceptance covers native Reject, session grants, remembered grants and
revoke; repeat, paste, modified and stale physical approval; exact ConPTY text
insertion without Enter; real named pipes and a compiled child client;
malformed registration and scope denial; two-host and connection isolation;
Unicode revision edits and undo; and shutdown with queued transport work.
Independent Astra review has no findings.

Final validation is green: `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings`, and
`cargo test --locked --workspace --no-fail-fast` all exit zero. Focused context
validation passes 14 cases with two ignored. Full summaries
report 2,821 core tests passed with 26 ignored and 66 bin tests passed with 29
ignored; every integration target is green, including seven plugin-worker and
six native LSP cases. Native CI acceptance remains pending.

## Accepted package: 5f private Windows context bridge

`34f53b6` adds the Windows adapter for the separately versioned Python MCP
bridge without changing its MCP or context tool schemas. The adapter resolves
the default context store from the OS LocalAppData known folder and keeps it
separate from plugin state. It opens existing local NTFS storage component by
component, rejects reparse points and linked identity files, and requires an
explicit current-user owner with a protected owner-only full-access ACL before
reading a credential. Windows discovery output is bounded through an owned
overlapped pipe and a hidden child process, including cancellation when a
descendant retains stdout.

Named-pipe connection admission validates the strict endpoint family, the
complete numeric registration, the actual pipe server PID, its exact nonzero
creation time and the retained server process's account SID before sending a
credential. Reads and writes use duplicated handles and bounded overlapped
owners. Each owner alone submits, cancels, drains and releases its operation;
interruption is deferred until owner completion. Unconfirmed cancellation
retains a bounded owner and disables later native I/O. One absolute exchange
deadline covers partial writes, and a mutation is known not sent only when no
frame byte was transmitted. Process liveness and connection reuse rely on the
retained process handle rather than the process exit-code sentinel.

Windows acceptance adds adapter cases for bounded and oversized discovery,
timeouts and descendant-held stdout. The ignored real-host case
`workspace::host::context::transport_tests::python_mcp_bridge_uses_real_native_host_security_unicode_and_revocation`
launches the Python MCP client against the private Rust named-pipe host. It
checks pre-authentication rejection, private credential admission, exact MCP
routing, Unicode reads and revision-checked edits, reconnect isolation and
native revocation. The Windows CI job now requires both the Python adapter
suite and that exact ignored real-host case to run and pass; it supplies the
Python executable explicitly and rejects a skipped acceptance test.

Independent Astra review accepted the storage, process proof, overlapped I/O,
cancellation, deadline and mutation-outcome ownership. Local validation passed
`cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`.
The full workspace suite had one unrelated `git_provider` blame test failure;
that exact test passed immediately in an isolated rerun. This machine had no
usable Python, uv or WSL runtime, so neither the Python suite nor the ignored
native Python acceptance is claimed locally. The required Windows CI gate is
the acceptance authority for those cases and remains pending.
The immutable/frozen and current Node/plugin conformance suites remain separate
required CI lanes; neither was run or claimed by `34f53b6`. The existing Node
readiness issue remains open in `context/issues/node_conformance_readiness.md`.

## Accepted package: 5g public Windows context access

Local source commit `a79e765` (`Enable public Windows context access`) starts the
Windows context service during normal workspace startup and opens the native
`:context-access [identity]` review and bounded `--context-list --json`
discovery. Grants, scope changes, remembered access, terminal-text proposals and
revocation retain the accepted physical-input and exact-identity rules.
Revocation disconnects the identity and removes its durable grant. This package
does not open interactive persistent attachment, switching or plugin startup.

The public acceptance uses the actual Runyte executable in a real ConPTY. It
physically grants and revokes access, exercises Unicode unsaved reads and
revision-checked edits, rejects stale and cross-connection handles, restarts a
remembered grant under a fresh host incarnation, and checks exact publication
retirement. The production Python bridge discovers through the actual
`runyte --context-list --json` route rather than injected discovery. A separate
post-service frontend-failure case pins the published process identity, releases
the injected failure, waits for natural joined exit, and proves that the exact
host publication was removed.

Every Windows editor, host and compiled wrapper fixture now receives its own
absolute `RUNYTE_CONTEXT_HOME` and fixture-owned `XDG_CONFIG_HOME`; the context
acceptance also pins its cache and runtime environment inputs. Fixtures execute
the repository binary or an already compiled libtest helper rather than a file
written by a test, and temporary storage remains owned until the relevant
process and transport owners have joined. README, the user guide, keymap
reference and Python bridge documentation describe the public boundary. Windows
CI requires the Python adapter, private real-host bridge and public real-executable
acceptance cases to run rather than skip.

CI run
[35821577813](https://github.com/runyte/runyte/actions/runs/35821577813) for the
previously pushed `cbc850e` passed every non-Windows job. Its Windows job exposed
a race after successful discovery output: an overlapped read could first pend,
then `GetOverlappedResult` returned `ERROR_BROKEN_PIPE` (109) after the child
exited. `a79e765` fixes both native read completion paths to treat read-side 109
as EOF while preserving bounds, the absolute deadline and process cleanup. The
focused four-case Windows Python adapter suite passes after that repair.

Final Astra review reports no findings. Local validation passes
`cargo fmt --check`, serialized
`cargo clippy --all-targets --locked -- -D warnings`, and serialized
`cargo test --locked --workspace --no-fail-fast`. The focused post-service
cleanup acceptance also passes.

CI run
[35832182423](https://github.com/runyte/runyte/actions/runs/35832182423) passed
all 17 non-Windows jobs. Its Windows job exposed five acceptance defects. The
terminal-job peer fixture used a nested `cmd.exe` that could exit before peer
identity was pinned; the durable context-storage fixture created its simulated
interrupted directory without the required private ACL; and a Windows host
could close a refused pipe before the frontend read the terminal refusal. The
Python bridge validated a 64-character `workspace_id` even though Runyte
publishes the stable 32-character workspace identity, so it filtered the real
registration before probing. Finally, the CI step exited after the private
bridge failure and did not run the public exact acceptance.

Remediation source commit `05935a6` (`Fix Windows context acceptance
regressions`) uses a readiness-signalled compiled child for the PTY peer,
creates the interrupted storage child through private storage, retains refused
Windows peers through reader release or one deadline, validates the real
32-character workspace identity, and accumulates the Python, private-host and
public-executable gate results before failing the CI step. Its focused bridge
test accepts 32 characters and rejects 31, 33 and 64.

Validation after `05935a6` is green: `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings`, and
`cargo test --locked --workspace --no-fail-fast` all pass. Focused storage,
PTY, shared transport and native frontend acceptance also pass. The exact
`public_windows_context_round_trip` and private real-host Python bridge cases
each selected one test and passed with the isolated
`target/python-3.13.15/python.exe` runtime under a temporary `.pth`/import
setup. This is local acceptance evidence; replacement Windows CI remains the
authority for the installed-Python route.

Replacement CI run
[35837383495](https://github.com/runyte/runyte/actions/runs/35837383495) at
`d693832` completed successfully with all 18 jobs green. Native Windows passed,
including the repaired Python adapter, private real-host bridge and public
real-executable context acceptance. The public Windows context checkpoint and
its remediation are pushed and remotely accepted through `d693832`.

## Accepted public Windows plugin lifecycle

Commit `9743bd9` opens configured plugin discovery, startup, stop, restart and
configuration reconciliation in standalone and host-owned Windows workspaces.
The existing worker, durable state and private handoff implementations retain
their protocol, approval and uncertain-outcome contracts. Windows hello omits
the unsupported `processes` capability: a plugin requiring it fails
registration, while one declaring it optional runs without that grant. Native
managed helper processes remain a separate implementation package.

Local acceptance passed `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings`, and
`cargo test --locked --workspace --no-fail-fast`. The real-process
`plugin_windows_worker` acceptance ran all eight cases, including public
registration, required-capability refusal, manager discovery, stop/reap,
restart with a new process and shutdown cleanup. The Windows Phase 1 command
availability tests were updated. Independent Astra review found no remaining
implementation findings. All jobs passed in
[CI run 35875460337](https://github.com/runyte/runyte/actions/runs/35875460337).

## Accepted Phase 2.5 explicit Windows attachment

Source commit `2563fba` opens explicit `-a` and `--persistent [WORKSPACE]` for
the native Windows frontend. A complete catalog resolves an ID, name or path;
a live selection keeps its exact publication, while a stopped or new workspace
starts a host through the provisional native job before attaching. Bare `-a`
discovers the current project or initializes the exact current directory.
`--project-root` is honored from a nested directory. An integrated terminal
uses its authenticated ParentAttach route instead of attempting detached
startup inside its ConPTY job. Occupied attachments refuse takeover, and
verbosity or log options on an existing host report that its logger is retained.
At this source checkpoint, public persistent wait, manager visits, in-editor
switching and automatic `workspace.mode: persistent` startup remained gated.

Local acceptance passed `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings`, the focused
`windows_public_attachment` real-process test, and
`cargo test --locked --workspace --no-fail-fast`. The inherited job on this
development host denies detached CreateProcessW startup. The test verifies that
exact policy refusal and absence of a new publication, then uses a fixture-owned
foreground host to check real interactive attachment, retained unsaved edits,
single-TUI ownership, nested `--project-root` and the retained-logging notice.
Successful detached creation remains unproven locally. Its
[CI run 35881502895](https://github.com/runyte/runyte/actions/runs/35881502895)
was cancelled by the next push; the later configured-mode run includes this
ancestor source. Independent Astra review found no remaining implementation
findings.

## Accepted Phase 2.5 configured bare attachment

Source commit `378a22b` routes an implicit, targetless Windows launch with
`workspace.mode: persistent` through the accepted exact native attachment or
authenticated ParentAttach path. It requires a discoverable current project or
explicit `--project-root`; if neither exists, it reports the missing project
without creating a workspace. Explicit `-a` retains its exact-directory
initialization behavior. `--standalone`, `--wait`, `--init` and file or directory
targets retain standalone semantics. Configuration is loaded once for the
implicit mode choice.

Local `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
focused real-process `windows_public_attachment` acceptance and
`cargo test --locked --workspace --no-fail-fast` passed. The fixture verifies
configured bare attachment to retained edits, standalone override and
target-bearing launches while a persistent TUI is occupied, and noncreating
refusal outside a project. The local inherited job still prevents a successful
detached startup, as recorded above. Independent Astra source review found no
remaining findings. All jobs passed in
[CI run 35883007224](https://github.com/runyte/runyte/actions/runs/35883007224).

## Implemented Phase 2.5 guarded CLI restart; acceptance pending

Source commit `8a8987c` opens Windows `--session-restart [WORKSPACE]` on the
native CLI. It selects one exact live publication from the complete catalog;
an omitted selector uses a discoverable current project without initializing
one. An ambiguous selector, stopped session, stale selected publication or
different configured namespace is refused without path, name or PID fallback.
Before stopping, a bounded `--version` child exercises the same detached
process/job launcher, then its process and job are reaped. A confirmed stop
precedes replacement startup. Normal stop preserves protected state; `--force`
requests its loss. Startup cancellation retains provisional cleanup ownership,
and an unsuccessful replacement after confirmed stop is reported without
claiming a running replacement. This package does not add a restart action to
the standalone session manager.

Local `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
the focused real-host `windows_session_cli` target (10/10), the
`release_packaging` target (7/7), and
`cargo test --locked --workspace --no-fail-fast` passed. The real-host test
proves that this runner's inherited job rejects the detached preflight with
CreateProcessW access denial before the selected host is stopped; its exact
ready publication remains unchanged. A two-host ID-prefix case proves CLI
ambiguity refusal without stopping either host. The test's successful restart,
protected-state refusal and force-replacement branch is conditional on a
runner that permits detached creation and was not exercised locally. That
stop-to-start acceptance remains an explicit gate. Independent Astra source
review is clear. All jobs passed in
[CI run 35888094832](https://github.com/runyte/runyte/actions/runs/35888094832).

## Accepted locally: native Ctrl-backslash and public integrated-parent wait

`eff117c` normalizes the native ConPTY `0x1c` Control key record to `Ctrl-\`;
`1159418` verifies a second press enters review and `i` returns to terminal
input in a real editor. The issue was moved to resolved in `e34d04c`.
Physical keyboard and layout behavior remain unverified.

`fee4819` routes Windows `--wait FILE...` from an authenticated persistent
integrated terminal through the existing private ParentWait authority. Files
resolve from the invoking terminal's directory, and the caller waits until all
requested buffers complete. Copied, stale, malformed and standalone parent
markers refuse; an ordinary shell with no parent context retains standalone
`--wait`. Real ConPTY acceptance covers two-file completion and restoration of
the origin terminal after client exit. Native formatting, all-target Clippy,
focused Windows tests and the full workspace suite passed; independent source
review is clear. These commits are local: automatic approval review blocked
push, so remote CI acceptance is pending.

## Accepted locally: selected Windows manager visit

`92329b0` opens Enter and Tab > Open on a selected compatible running row in
the persistent Windows session manager. The App freezes `WorkspaceSelection`;
the native service proves that exact live publication before switching, with no
path, name or PID fallback after replacement. Stopped and incompatible rows
refuse, and the standalone manager stays control-only. The real two-host
ConPTY acceptance visits a retained destination, restores the dirty source,
then verifies a replaced selected publication is refused while both hosts and
the source state remain intact. Focused App tests, native formatting, all-target
Clippy and the full workspace suite passed; independent Astra source review is
clear. The commit is local after automatic approval review denied the push; no
remote CI result exists for it.

## Next Phase 2.5 gates

Successful CLI restart stop-to-start, protected-state refusal and forced
replacement still need a detached-capable Windows acceptance run. Persistent
wait into a host from an ordinary shell, numbered and other in-editor
navigation, directory handoff and combined Git
branch/worktree removal remain closed until their own acceptance gates pass.
Keep the native ConPTY job limits and exact publication authority in those
packages.

## Phase 2.5 implementation order

The [catalog/service design](../plans/active/WINDOWS_CATALOG_SERVICE.md) records
the reviewed integration map and acceptance requirements. DiscoveryScope, which
that design calls a prerequisite, is already applied as described above.

1. Completed in 4e.4a and 4e.4b: native control orchestration and CLI for
   list/rename/stop/stop-all/clean
   against exact retained publications. Stop success requires actual process
   exit, not an acknowledgment or missing ready file. Aggregate stop-all failures
   while attempting other distinct hosts. The guarded CLI restart route is now
   implemented in `8a8987c`; its detached-capable real-host stop-to-start
   acceptance remains pending. Extract pure CLI table presentation while
   preserving Unix behavior; selector-only commands must not invent a project.
2. Shared typed row selection and an owned native catalog service. Duplicate
   same-project publications require separate selection, preview, prompt and
   completion identities. A stale publication key must never fall back to a path,
   name or PID. Keep native proof in the service, not display DTOs. Use one owned
   runtime/thread, bounded admission/events and joined shutdown; App holds only
   a sending handle. Missing Git remains supported during worktree discovery.
3. Accepted foundations: process-exit supervision, private native frontend
   attachment, exact switching, ParentWait and ParentAttach. Explicit public
   `-a`/`--persistent` attachment is accepted in `2563fba`, configured bare
   attachment in `378a22b`, integrated-parent public wait in `fee4819`, and
   selected persistent-manager visits in `92329b0`.
   The CLI restart implementation is committed in `8a8987c` with successful
   replacement acceptance outstanding.
   Preserve common editor semantics, protected shutdown and connection-owned
   waits while opening the remaining public gates.
4. Completed in 4e.5j5: ParentAttach authorization and routing using the
   [reviewed parent design](../plans/active/WINDOWS_PARENT_ROUTING.md). The
   retained pipe peer's exact ConPTY job membership, terminal capability and
   current attachment ownership agree through settlement. Destination startup
   remains outside the requesting terminal job, and ConPTY job limits are not
   relaxed for detached startup.
5. Acceptance and documentation, including combined Git branch/worktree removal
   through the now-native persistent coordinator.

Detached and foreground native hosts have real process tests. The host owns one
internal physical-input attachment; private ParentWait and ParentAttach and the
explicit and configured bare public attachment routes are accepted. Guarded
CLI restart has local refusal evidence but awaits detached-capable stop-to-start
acceptance. Integrated-parent public wait has local real-process acceptance;
ordinary-shell persistent wait and in-editor navigation beyond the selected
manager visit remain gated.

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

## Phase 2.6 status

Native plugin worker/process ownership and framing is accepted in `ce888ee`, and
durable plugin state is accepted in `140347e`. Private Windows `terminal.open`
and `external.open` handoffs and physical-frontend approval are accepted in
`8148437`. The context sequence is accepted from native identity and remembered
grant storage in `61acb18`, through transport and discovery in `439f4da`, host
grants in `bbdb12a`, and the Python MCP bridge in `34f53b6`. Source commit
`a79e765` opens normal Windows context startup, `:context-access` and
`--context-list --json` with the public acceptance described above. Remediation
`05935a6` corrects the Windows acceptance failures exposed by CI run
35832182423, and green replacement CI run 35837383495 remotely accepts the
context sequence through `d693832`.

At the context checkpoint, public plugin discovery, startup, stop and restart
were the remaining 2.6 gate. Source commit `9743bd9` has since opened that
lifecycle with local native acceptance and independent review, preserving worker
ownership, durable state, physical approval and uncertain-outcome contracts.
Windows managed helper processes remain unavailable. The public context
sequence is remotely accepted; the public plugin lifecycle passed every job in
[CI run 35875460337](https://github.com/runyte/runyte/actions/runs/35875460337).

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
configuration/cache/state. Windows fixtures must additionally set an absolute,
fixture-owned `RUNYTE_CONTEXT_HOME`; XDG configuration does not isolate the
separate LocalAppData context store. Use compiled or checked-in executable
fixtures;
never execute a program written by a test. Retain process/job ownership through
cleanup before deleting fixture storage. Native process checks may require the
normal token outside the sandbox, as in preceding acceptance runs.

The coordinating agent has no pending local validation command. Public context
source, remediation and handoff commits through `d693832` are pushed and
accepted by CI run 35837383495. Plugin lifecycle source commit `9743bd9` passed
CI run 35875460337. Direct attachment source commit `2563fba` passed local
validation; CI run 35881502895 was cancelled by the next push. Configured bare
attachment source commit `378a22b` passed local validation and every job in CI
run 35883007224. Guarded CLI restart source commit `8a8987c` passed local
format, Clippy, focused CLI and release packaging tests, and the full native
suite, but successful replacement acceptance remains pending on a runner that
permits detached creation; CI run 35888094832 passed every job. Native
Ctrl-backslash source `eff117c` and acceptance `1159418`, resolved issue record
`e34d04c`, public integrated-parent wait source `fee4819`, and selected
persistent-manager visit source `92329b0` passed local validation and remain
unpushed after automatic approval review blocked the push. No remote CI result
exists for those commits. Do not retry the denied push
without explicit approval.
Inspect Git status and preserve unrelated working-tree edits before committing.
