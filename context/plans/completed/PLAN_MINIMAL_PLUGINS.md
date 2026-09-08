# Experimental process plugins

## Investigation and scope

`command.rs` has closed, copyable command identities, typed validated invocations,
and a static colon inventory. `App::execute` is the shared semantic dispatcher.
The configured keymap currently moves existing bindings only; dispatch, help and
hints consume the same `Keymap`. Runtime identities and owned discovery metadata
are missing. `headless.rs` is deliberately a test facade, not an extension API.

`WorkspaceHost` owns `App` in both modes. `start_host_services` runs after the
first standalone frame and once in the persistent host, not in attached clients.
The event loops already wait on Tokio service receivers. Plugins can use that
ownership and event-driven scheduling without a timer or disabled startup work.

Host buffer IDs encode append-only buffer slots; closed slots are tombstones.
Revisions increase on mutation, including undo. `execute_expected_command`
requires a prepared frame and active buffer because interactive operations may
use geometry. `apply_expected_transaction` instead resolves an explicit live
buffer, checks revision and bounds, and calls `App::apply_to_buffer`. That path
updates syntax/LSP and maps all panes' selections. `Transaction::new` silently
drops overlapping changes: extension validation must reject overlaps *before*
construction. Open insert undo groups must be committed before a plugin edit to
make the result a separate checkpoint. Selection anchor/head describe direction;
`operative_spans` translates the editor's inclusive caret behavior into actual
half-open replacement spans.

## Decisions

Implement one external-process runtime with newline-delimited bounded JSON and
an exact `runyte-experimental-1` handshake. Existing Tokio process/IO and serde
JSON avoid new dependencies. Embedded Lua would require an interpreter and a
careful instruction/memory cancellation boundary in the editor process. WASM
would add an engine and host ABI, compilation/startup and packaging work. Neither
is justified for a selected-text transformation. Process failure isolation is
not a security sandbox: enabled programs run with the user's permissions.

Configuration explicitly enables a named executable plus argument vector; no
workspace discovery, implicit loading, shell command expansion or package
manager. Limit enabled instances, command registrations, message bytes, captured
buffer size/selections, pending invocations, queued messages and execution time.
Start once per host service lifetime. Failure removes commands/bindings and
pending work, reports retained feedback and kills the process. No automatic
restart loop. Host shutdown cancels workers; TUI detach does not. Configuration
changes take effect on the next host start.

The wire contract is separate from `protocol/`, core types and `headless`.
Version names are experimental epochs: incompatible changes require a new exact
handshake version and documentation; no compatibility across epochs is promised.
Identifiers are opaque strings valid only in the current process connection and
host lifetime. Offsets count Unicode scalar values, never bytes, UTF-16 or screen
columns. Capture invoking buffer/revision/text, primary selection, anchor/head
and half-open operative spans before enqueueing. The initial operation replaces
each captured span with one returned string; plugins cannot run arbitrary
interactive commands or choose a new target after dispatch. Validate count,
size, liveness, revision and non-overlap atomically. Apply one transaction through
the existing mutation path without activating the target or preparing a frame.
Return structured accepted/completed/error information with invocation identity
and resulting revision; stale and closed results cannot write anything.

Runtime commands use `plugin.<plugin-id>.<local-name>`, cannot replace built-ins,
and use host-monotonic internal IDs so removed invocations never alias a later
registration. Register a bounded command set during the handshake. Resolve colon
invocation and completion from the same runtime metadata used by the keymap.
User-configured optional bindings are admitted through effective-scope validation
for both fast-pane variants; collisions are refused rather than overriding keys.
Cleanup removes bindings and metadata together. No default key changes.

Subscriptions observe explicit invoking buffer handles. Subscribe acknowledges a
baseline revision; subsequent host turns emit revision/closure observations in
host order, including edits, undo and reload. Observations may coalesce multiple
changes within a turn and are not a replayable edit log. Every delivered event
has a connection-monotonic sequence. Unsubscribe acknowledges after prior queued
events and prevents later events. Queues are bounded: slow consumers fail and
lose registrations rather than stalling input or silently dropping events.

## Implementation and validation sequence

1. Add wire values, bounded process worker, host-owned lifecycle, captured command
   submission and explicit-target application adapter.
2. Integrate runtime identities and owned metadata with semantic execution,
   colon discovery and the existing validated keymap; attach service events to
   both event loops.
3. Add subscriptions and cancellation/failure cleanup, runnable Python example,
   installation/API documentation and current behavior reference updates.
4. Test multiple selections, Unicode/direction, atomic validation and one-step
   undo; stale/closed/inactive targets; registration collisions and removal;
   bounded event ordering/unsubscribe; malformed/failed/timed-out plugins; host
   attachment lifetime and disabled startup. Use isolated temporary storage and
   the checked-in executable stand-in for process tests.
5. Run fmt, warnings-as-errors Clippy, cargo test and canonical llvm-cov; retain
   the 89% floor. Measure release startup and idle on this Linux host and record
   the result and native macOS validation limitation. Review the final diff and
   move this record to completed only when implementation is finished.

## Delivered architecture

The implementation uses `src/plugin.rs` for independent JSON values and a Tokio
process worker. Workers live in `WorkspaceHost`; `App` retains only registration
metadata, bounded send handles and captured pending context. Host service startup
creates them once, and both event loops receive their events. No new dependency,
plugin discovery scan or timer was introduced.

`CommandId::Plugin` and `BindingTarget::Plugin` carry monotonically allocated
host-lifetime identities. Runtime metadata supplies colon parsing/discovery and
owned binding descriptions, while borrowed palette descriptors preserve the
actual runtime identity without leaking strings into static allocations. Both
configured keymap variants are validated atomically, including the grammar's
reserved count and cancellation keys. Cleanup rebuilds both variants without
removed entries. A host-supplied `plugin.<id>.stop` command cancels and removes an
instance. Old command identities cannot address a later registration.

The wire operation deliberately returns replacement strings for captured spans,
rather than accepting arbitrary edits or exposing interactive commands. Native
anchor/head direction is retained alongside authoritative half-open operative
spans. The adapter commits a preceding insert undo group and calls the existing
revision-checked host transaction operation. Syntax, LSP and every pane's current
selection continue through the existing reconciliation path. Completion feedback
is tied to the invoking action ID so it cannot overwrite a newer action echo.

Subscriptions observe only previously issued buffer handles, with one watch per
buffer. Baseline acknowledgements and connection-ordered revision/closure events
share the bounded output queue. They coalesce changes within a host turn; this is
an invalidation API, not a transaction journal or background text-read API.
A result handler rechecks instance membership after observation delivery because
backpressure or an explicit stop can retire that same instance at this boundary.
The worker retains partially read input across outbound writes, preserving JSON
framing while serving both directions.

Limits and error codes are specified in `docs/plugins.md`, including a 256 KiB
snapshot, 1,024 selections, 512 KiB replacements, 64 issued buffer handles,
8 enabled processes, 16 plugin commands, one pending invocation per process,
1 MiB wire messages, bounded queues and registration/execution/write deadlines.
Direct child termination is asynchronous; descendants and OS CPU/memory limits
remain the plugin's responsibility. Process isolation is not a sandbox.

The runnable `docs/plugins/uppercase.py` example requires only Python 3. No
marketplace, package installation, second runtime, general command automation,
background read operation, live reload or arbitrary plugin UI was implemented.
The current API and practical enablement/invocation instructions are documented
in `docs/plugins.md` and linked from the README and user guide.

## Validation and completion

On native Linux, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
and `cargo test` passed. The suite passed 3,079 tests with 33 existing ignored
tests. Canonical `cargo llvm-cov --locked --workspace` passed the same suite and
reported 91.76% total line coverage, above the unchanged 89% floor. Detailed
counts and coverage boundaries are in `context/reference/test-coverage.md`.
Normal process, PTY and local-socket access was used; the initially sandboxed
persistent-host run could not bind its socket and was rerun with that access.
Native macOS validation remains for CI or a macOS host.

Focused plugin tests live in `src/workspace/host/tests/plugins.rs`. The real
persistent attachment test is
`plugin_completion_while_detached_and_reattachment_keep_one_process` in
`tests/persistent_host.rs`. The runnable Python example passed an independent
handshake and Unicode multiple-selection smoke test. Test-created process names
link to the checked-in stand-in and keep behavior in adjacent data files.

Release startup and idle were compared against a retained `ef1bf58` binary using
the existing PTY harness. First-document medians stayed close across the three
representative fixtures; all configurations had zero median idle CPU and zero
screen writes. One disabled-plugin window recorded 0.10% CPU. The complete
medians, ranges, measurement limits and repeatable `benchmarks/plugins.py` command
are retained in `context/reference/startup-performance.md`. This measures the
shipped example, not arbitrary plugin work or registration readiness.

The requested vertical slice is complete. Later work may consider additional
operations or runtime policy, but this record authorizes no second runtime,
stable general automation API, marketplace, package manager or plugin UI.
