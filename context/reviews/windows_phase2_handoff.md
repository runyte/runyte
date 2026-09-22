# Windows Phase 2 continuation

Checkpoint: 2026-09-22, branch `feat/windows-support`, package 4e.5c owned
native catalog service accepted in `92e44dc` (`Own native catalog service and
retained controls`). The preceding manager selection package is `6befd91`.
This record supplements the [active plan](../plans/active/PLAN_WINDOWS_PHASE2.md)
with the working-tree state and immediate continuation steps. Read this record
before the older chronological progress entries. No previous chat is required.

## Scope and delivery

Phase 1 is complete. Phase 2.1 through 2.4 and the native Ctrl+h/Ctrl+j correction
are complete. Phase 2.5 is in progress; Phase 2.6 remains to implement. Integrated
Git is optional: missing Git must leave the integration disabled without failed
spawn loops or runtime errors. Combined branch/worktree deletion still needs the
native persistent-session coordinator; separate guarded operations work.

Continue sequential work packages with an independent subagent review after
each package. Incorporate findings and repeat review until none remain before
advancing. Commit and push accepted checkpoints to `feat/windows-support`.
The user authorized the specific push through `5947fbb` to
`git@github.com:runyte/runyte.git`, and that push succeeded. Subsequent local
checkpoints are not included in that authorization; ask before pushing them.
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

## Next package: supported native manager controls

Connect the accepted native service handle and semantic events to the existing
session manager for supported list, preview, rename, selected stop and clean
actions. Keep unsupported attachment, switching, current-session markers,
number shortcuts and destination navigation gated until frontend ownership
supplies actual current publication identity. Preserve exact selected-row
identity through actions and complete events, and keep colon user selectors
separate. Test duplicate same-project live rows and stale publication refusal
with real native hosts before exposing the manager control actions.

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
3. Process-exit supervision and native frontend attachment/switching/waits.
   Preserve common editor semantics, protected shutdown, and connection-owned
   waits. Complete real-host tests before opening the public availability gates.
4. Parent-terminal authorization and routing using the
   [reviewed parent design](../plans/active/WINDOWS_PARENT_ROUTING.md). Actual
   retained pipe-peer membership in the exact ConPTY job, a terminal capability,
   and current attachment ownership must agree. Metadata PID, inherited marker
   text or ancestry alone is insufficient. Prepare destinations from the parent
   host's owned worker outside the requesting terminal job. Do not relax ConPTY
   job limits to enable detached startup.
5. Acceptance and documentation, including combined Git branch/worktree removal
   through the now-native persistent coordinator.

The internal detached Windows host already exists and has real process tests;
foreground host supervision and the interactive native frontend remain absent.
The current native host intentionally refuses physical-input, attachment and
parent requests until their ownership paths are accepted.

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
Keep the captured runtime alive through teardown; retain the bundle rather than
risk use-after-free if unregister unexpectedly fails.

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

Implement native plugin worker/process ownership and framing, durable state,
handoffs and approval ownership, then context transport/grants and the Windows
Python MCP bridge. Preserve existing protocol bounds and physical-frontend-only
approval. Durable plugin state needs an explicitly provisioned OS storage anchor;
do not introduce a broad user-directory fallback. Validate immutable and current
Node/plugin conformance plus real Windows context clients.

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
