# Windows Phase 2 continuation

Checkpoint: 2026-09-21, branch `feat/windows-support`, implementation commit
`221a48d` (`Add exact native session discovery and guarded history updates`).
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
Report completion of the entire Phase 2 or an unexpected blocker requiring a
decision. There is no current decision blocker. Do not enable public persistent
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

## Uncommitted applied package: 4e.3e discovery scope

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

Final independent review has no findings. Fifteen location tests and 34 catalog
tests pass, including eight new scope regressions. Formatting and all-target
Clippy pass. The full native suite has not yet run on these uncommitted changes;
combine that acceptance with the next reviewed naming package. Do not describe
the full 3,254-test result as a result for this working tree.

Local diagnostic logs are `target/windows-discovery-scope-location-tests.log`,
`target/windows-discovery-scope-catalog-tests.log`, and
`target/windows-discovery-scope-clippy.log`. They are disposable evidence, not
required development records.

## Next package: 4e.3f authoritative stopped names

The [reviewed stopped-name design](../plans/active/WINDOWS_STOPPED_NAMES.md)
contains the authority, ordered locks, bounded recovery and acceptance contract.
Its implementation is staged locally under
`target/windows-stopped-names-package/`, with a mapping in its README. It is
**not applied to source, compiled, or natively tested** at this checkpoint.
Final independent review by `review_shell_recovery` has zero remaining findings.
The author was `review_ci_limits`; a new session can use new reviewers.

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

Before applying, compare every destination with its `.before` baseline, allowing
only CRLF/LF differences. Preserve unrelated changes and the existing history
transaction module. Write fresh destination bytes: a previous Copy-Item preserved
an old modification time and Cargo reused stale output. Do not regenerate older
staging scripts over reviewed snapshots. If the ignored staging directory has
been removed, implement from the retained design and repeat independent review;
the durable contract does not depend on that directory surviving.

The sixteen staged fixtures cover noncreating stored-name reads; stored/live/cache
precedence and default reservation; exact live and hidden metadata; stale intent,
forgotten history, configured collisions and startup serialization; exact occupied
or malformed records; committed rename despite cache failure; foreign replacement,
source chains, retained recovery, partial staging/restoration and owner teardown.
Review corrections preserve the original native error source, reject a changed
cache fallback when no stored authority exists, gate the now-unused shared helper
to Unix/tests, and actually persist fixture names before testing authority.

After applying, run the two focused filters
`workspace::windows_endpoint::names::stopped::tests` and
`workspace::windows_catalog::history::stopped_names::tests`, then existing endpoint
and catalog tests. Complete formatting, Clippy and the full workspace suite,
review any corrections, update acceptance evidence, then commit and push both
accepted packages. Keep the Windows support issue open until the entire scope is
resolved; issue resolution requires the repository's separate follow-up commit.

## Remaining 2.5 implementation order

The [catalog/service design](../plans/active/WINDOWS_CATALOG_SERVICE.md) records
the reviewed integration map and acceptance requirements. DiscoveryScope, which
that design calls a prerequisite, is already applied as described above.

1. Native control orchestration and CLI: complete list/rename/stop/stop-all/clean
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
does not require a live agent to finish a transaction. Uncommitted source and
ignored staging remain local to this checkout; a different checkout will need those changes transferred
or reconstructed from the retained contracts. Inspect git status before editing.
