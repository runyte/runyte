---
title: "No durable diagnostic log survived a process failure, most visibly in persistent mode"
status: resolved
reported: 2026-08-27
resolved: 2026-08-27
commit: 5a3b9ea
---

## Resolution

Commit `5a3b9ea` (`Add durable diagnostic logging owned by the editor
process`) adds a bounded local log written by whichever process owns `App`.

The whole mechanism is `src/log.rs`, a module the rest of the editor does not
type against. `Logger::start` takes a `Settings` value — level, role, workspace
identity, PID — and a `Sink`, opens or rotates the file before returning so an
unusable destination is reported to its caller rather than discovered later by
a background thread, and spawns one writer thread behind a bounded
`sync_channel`. `Logger::emit` formats a record and `try_send`s it: a full
queue increments a dropped counter and returns, so no producer ever waits for
disk, and the writer emits a summary record when it notices the counter has
moved. `Destination::File` tracks its own size and rotates when the next line
would cross `MAX_LOG_BYTES`, keeping one previous file named by appending `.1`;
`open_log_file` performs the same rotation at startup, which is what stops a
frequently restarted host from inheriting a full file forever. `append_sanitized`
replaces control characters so an operating-system error string embedded in a
message cannot split one event across two lines.

Instrumentation goes through `log_error!`, `log_warn!`, `log_info!`,
`log_debug!`, and `log_trace!`. Each checks one atomic before formatting and
takes an optional `; "key" => value` tail for structured context, which is how
compact identifiers — workspace ID, language, server generation, connection,
Git request, terminal session — reach a record without any domain type being
serialized.

`default_path` is where ownership lives: `Role::Host` resolves to `host.log`
and `Role::Standalone` to `standalone-<pid>.log`, both beneath the resolved
runtime state root. The PID is what guarantees two concurrent standalone
editors cannot write or rotate one file, so no cross-process append lock was
needed. A client installs no logger at all, which is what keeps transport
diagnostics from depending on the transport being healthy.

`src/main.rs` installs the logger immediately before `App` is constructed,
choosing the role from `LaunchMode::Serve`, and installs the panic hook there
too. `initialize_logging` returns the failure text for an unusable default so
the notification can be pushed once `App` exists, and turns an unusable
explicit `--log` into a startup error. `HostStartup::with_logging` passes
verbosity and destination to every host this process starts; the paths that
find one already running call `report_retained_logging` instead, so `-v` or
`--log` on an attachment says the session kept its configuration rather than
appearing to have changed it. No runtime log-level command or protocol message
was added.

`App::logging_health` projects `log::status()` onto a `log` row in
`:service-health`, through the free function `logging_health_entry` so both a
healthy and a degraded logger are coverable without installing a process-wide
one in a test. `App::open_log_buffer` flushes, reads the owning process's file,
and opens it through the existing `open_virtual_page` path under the new
`GeneratedViewIdentity::Log`, so `[log]` is an ordinary read-only generated
buffer.

Two boundaries turned out to be recording nothing at all, and the fix covers
both rather than adding a record beside them. `publish_attached_frame` cleared
`active` silently when a client's channel was already closed, which is the
common case for a client that goes away between frames; it now records the
departure, and the `ServerEvent::Disconnected` that follows finds no
attachment and stays quiet instead of reporting the same client twice.
`note_ended_service` records a background service whose channel closes exactly
once, in both the standalone and host loops, because the editor keeps working
without it and nothing else says so.

`workspace_id` moved from `src/workspace/transport.rs` to
`src/workspace/identity.rs`, where it is portable and public, so the transport
endpoint, the session catalog, and diagnostic records all derive one identity
rather than two implementations that could drift.

Tests. Unit tests in `src/log.rs` cover the verbosity mapping and its trace
cap, what each level admits, the one-line record shape and its role/PID/
workspace/target prefix, structured fields, the standalone/host path
derivation, rotation at startup and in flight with exactly one previous file
retained, a stalled writer that drops instead of blocking, and an unusable
destination reported at construction. `src/launch.rs` covers `-v`,
`--verbose`, the clustered `-vv`/`-vvv` spelling, and `--log` validation.
`src/app/settings_workflows.rs` covers the three service-health projections,
and `src/app/tests/presentation_and_settings.rs` covers `:log-open` in a
process with no logger. `tests/diagnostic_log.rs` drives real processes: a
standalone editor keeping warnings and errors and omitting the rest, repeated
`-v` reaching each documented level, two concurrent standalone processes
owning separate files, an invalid `--log` failing startup without falling back,
an unusable default leaving a host serving, a host owning `host.log` and
recording attachment, detachment, and disconnection while its client comes and
goes, `:log-open` showing those records back through the buffer, an attachment
reporting the retained configuration, a client-side failure appending nothing,
rotation across a host restart, a panic leaving its location and message
without changing process failure, and a redaction pass at trace level
asserting that file text, typed text, a terminal child's output, and an
environment value all stay out of the log.
`tests/release_packaging.rs` was adjusted: `--help` still may not use `host` or
`client` as vocabulary, but `host.log` is exempt, because help has to name a
path somebody can open.

A follow-up commit, `33aa45f` (`Keep typed text and subprocess output out of
diagnostic records`), fixes what a review of the above found. Two records
wrote content the report forbids, at the default level: `GitError`'s `Display`
quotes the argument vector — which for `git commit … -m <message>` holds the
message just typed — and Git's stderr, and `LspEvent::Stopped`'s message
embeds up to 8 KiB of the server's own stderr. `GitError::redacted` now keeps
only fields that cannot carry local content, and the language-server record
carries the language alone; both details still reach the person through the
notification, `:lsp-status`, and the interaction line. The redaction test drove
neither a commit nor a language server, which is why both got through; it now
starts a server that dies with a secret on its stderr and yanks a secret into a
register, and `GitError::redacted` has a per-variant test in `src/git/mod.rs`.
The same commit records the two remaining silent departures in
`send_active_response` — including a client that has stopped reading, the
report's "stalled writes" — marks the logger failed when its writer thread is
gone rather than reporting it healthy, gives the dropped-record summary the
shared record prefix, records forced termination and a terminal session that
fails to start, and corrects three assertions in `tests/diagnostic_log.rs` that
compared an empty file with an empty file or named the wrong default path.

A second review follow-up, committed with this record, closes five more
boundaries that the initial coverage did not exercise. `main` no longer writes
an arbitrary propagated `anyhow` chain into the durable file: its final record
states only that Runyte exited with an error, while `run_host_server` now
records listener loss and endpoint-cleanup failure at the boundaries that can
classify them. This keeps an environment-derived `RUNYTE_INPUT_TRACE` path out
of the log without losing the useful detail of a detached persistent-session
failure.

`serve_connection` had discarded its own error after an established stream
ended on malformed JSON, a truncated frame, or a failed write. It now emits a
transport-owned `ServerEvent::TransportFailure` before `Disconnected`;
`run_host_server` records that reason at warning level and sends no response on
the stream that just failed. This is deliberately a transport event carrying a
bounded reason, not a logging type or a new protocol message. The connection
loop is generic over its asynchronous stream only so the framing regression
test can use an in-memory duplex stream rather than silently skip when Unix
socket creation is unavailable.

The writer's first post-startup failure was already retained in `Status`, but
nothing surfaced it unless the person opened `:service-health`.
`unreported_failure` and `note_failure_reported` give the standalone and host
event loops a one-shot check on their existing periodic tick, which adds one
warning notification while editing or serving continues.

The original process-owned rotation argument applied only to default names.
Two processes given the same explicit `--log` path could still maintain
independent byte counts and rotate over one another. On Unix, an explicit sink
now takes a non-blocking advisory `flock`, with a two-second budget for an
exiting owner to hand the destination to its replacement; a concurrent owner
is refused with an actionable startup error. Explicit rotation copies the
completed file aside and truncates the still-locked active inode, avoiding a
re-lock window in which a second owner could slip in. Default files do not take
this lock because standalone names are unique and locking `host.log` would
obstruct the existing persistent-host restart handoff. This is the one
deliberate cross-process ownership mechanism beyond the report's default-path
design.

Finally, `prune_standalone_logs` bounds the files left by repeated standalone
launches. Each default standalone initialization retains the four newest logs
of exited processes, removes older active files and their `.1` siblings, and
never touches a live owner's files. Runtime state remains under the configured
workspace state root.

Tests. `tests/diagnostic_log.rs` adds
`a_second_process_is_refused_when_an_explicit_log_is_owned`, which holds one
real Runyte process after logger initialization and proves a second cannot
write the same `--log` destination;
`a_malformed_frame_is_recorded_in_host_log_at_the_default_level`; and
`the_top_level_failure_record_never_carries_a_propagated_error_chain`, which
first proves the environment-derived trace path reached stderr. Unit tests in
`src/log.rs` add
`stale_standalone_logs_are_bounded_without_touching_a_live_owner` and
`a_logger_failure_is_reported_once`. The existing
`malformed_established_messages_always_disconnect_host_state` test in
`src/workspace/transport.rs` now asserts the framing failure and the subsequent
disconnection in order, and
`an_established_stalled_peer_disconnects_and_releases_its_slot` asserts the
same ordering for a failed framed write. Each new assertion was also run
against its reverted behavior and failed at the intended boundary rather than
passing on an empty file, an impossible path, or a skipped socket.

A later CI-hardening follow-up fixes the process fixture rather than weakening
the five-second host-response boundary. `tests/diagnostic_log.rs` gave every
parallel test the same runtime registry and cache, made the host-log lifecycle
case infer `:log-open` completion from replaceable frontend frames, and dropped
an explicit detach before reading its `Detached` response. On slower Linux and
macOS runners the lifecycle case could therefore time out, while macOS could
also retain the resulting broken-pipe warning in a different assertion. Each
project now owns short private runtime and cache directories that are removed
with its fixture, detach assertions complete the protocol handshake, and
`a_host_owns_host_log_and_records_client_lifecycle_while_detached` reads the
opened `[log]` buffer through `ListBuffers` and `ReadBuffer`. The related
`attaching_with_logging_flags_reports_the_retained_configuration` test also
consumes the detach response, so it measures the host's retained log level
without manufacturing a transport failure.

A later rotation-fixture follow-up removes one remaining ordering assumption.
`rotation_bounds_the_host_log_across_a_restart` previously treated the
synchronous appearance of the rotated file as evidence that the asynchronous
writer had also persisted the subsequent publication record. The test now
requests a clean host shutdown, receives its protocol acknowledgement, and
waits for successful process exit before inspecting either file. Host exit
follows the logger's bounded flush, so the assertions describe final rotation
state without polling or sleeping for writer progress.

A later interactive-runner follow-up removes a controlling-terminal assumption
from `attaching_with_logging_flags_reports_the_retained_configuration` in
`tests/diagnostic_log.rs`. The test launched `runyte --persistent` with piped
standard streams and expected frontend initialization to fail, but Crossterm
can reopen `/dev/tty` when `cargo test` itself owns one. The child then entered
the TUI and waited indefinitely while writing terminal controls into the test
output. The fixture now connects through `--wait` with a deliberately rejected
binary target, which reaches the same retained-logging report through the
noninteractive control protocol and exits on the host's bounded refusal. Its
subprocess wait is itself capped at five seconds, so a later loss of that
response fails instead of hanging the suite.

The same correction covers the two other fixtures that treated redirected
standard streams as proof that no controlling terminal existed.
`detached_host_supervision_helper` in `tests/workspace_bulk.rs` now starts its
host through a rejected binary `--wait` request, which is the noninteractive
boundary that supervision test needs. The explicit `--persistent` regression
in `tests/persistent_host.rs` retains that launch path, but runs it through an
ignored helper that enters a fresh process session before `exec`, leaving the
multithreaded test runner untouched and making `/dev/tty` deterministically
unavailable. That helper is also bounded to the existing five-second host
response deadline.

Known limitations. An explicit `--log` is honoured only by processes that own
editor state. Passing it to a session-management command that neither starts
nor attaches to a session — `--session-list`, `--session-stop`,
`--session-clear-all` — is accepted and ignored without comment. The report
that a running session retained its logging configuration is an `eprintln!`,
so on an ordinary `runyte --persistent -v` it is printed immediately before
the alternate screen opens and the person is unlikely to see it; it is visible
on `--session-start` and `--wait`. Filesystem-watcher lifecycle beyond its
channel closing, and a host restart as distinct from a stop followed by a
start, are not instrumented.

## Report

Runyte has strong user-facing failure reporting but no general diagnostic log
that survives a process failure. The interaction line reports the immediate
outcome, `:notifications` retains bounded workspace-lifetime feedback,
`:service-health` describes optional-service state, and individual subprocess
boundaries retain useful details such as language-server stderr and labelled
Git output. Debug builds also have narrow opt-in input and startup-timing
traces. None of these provides a durable chronology of editor, service, and
transport lifecycle events.

This is most visible in persistent mode. A detached host owns the editor and
its asynchronous services after the client TUI leaves, and its ordinary stdout
is unavailable. If the host or a transport connection later fails, a client
may receive only the final error. The resolved
`workspace_ended_inside_a_message.md` report is a concrete example: the only
surviving evidence was `Error: workspace transport ended inside a message`,
which did not say which lifecycle event or framed write preceded it. Logging
would not replace the boundary checks and focused regression tests that fixed
that defect, but it would preserve the temporal context needed to choose the
right boundary to investigate.

### Expected behavior

Runyte has a small, bounded, local diagnostic log for warnings, errors, and
explicitly enabled verbose events. It complements notifications and health
reports; it is not a second user-facing notification system and is not an
audit trail.

Log ownership follows editor-state ownership:

- In standalone mode, the standalone process owns its log.
- In persistent mode, the persistent host owns the canonical workspace log.
  The host owns `App`, buffers, language servers, Git state, terminal sessions,
  notifications, and other durable workspace state, and its log remains
  available while no TUI is attached.
- A client never appends to the host's log and never forwards log records over
  the local protocol. Transport diagnostics must not depend on the transport
  being healthy, and a shared file must not acquire multi-process locking,
  interleaved rotation, or ambiguous lifecycle ownership.
- The host records client lifecycle facts it observes, including attachment,
  detachment, protocol refusal, disconnection, and transport failure. It does
  not claim to record failures confined to client-side input, rendering, or
  terminal setup.
- Client-only failures continue to be printed to stderr after terminal state
  has been restored. A separate opt-in `client-<pid>.log` may be added later if
  client-only diagnosis proves necessary, but it is not part of this issue and
  must never become the host log.

The canonical persistent log is `host.log` beneath the resolved runtime
workspace state root. Runtime logs belong under the configured runtime state
boundary, normally `.runyte/`, and never under the Git-tracked `context/`
directory. A standalone default must likewise be process-owned. Because more
than one standalone editor may open the same workspace, the implementation
must either derive a distinct standalone filename, such as
`standalone-<pid>.log`, or otherwise guarantee that two standalone processes
cannot write or rotate the same file. Cross-process append locking is outside
the intended design.

`:log-open` opens the log owned by the process that owns `App`. It therefore
opens the standalone process log in standalone mode and `host.log` in
persistent mode. It does not open or aggregate a client-side trace. The log is
an ordinary generated read-only buffer and must follow the established buffer
and generated-page vocabulary.

### Levels and startup controls

The default level is warning: warnings and errors are retained without asking
the person to reproduce an unexpected first failure, while routine operation
does not produce a high-volume trace. Repeating `-v` raises the startup level
through info, debug, and trace, capped at trace. `--log <path>` selects an
explicit destination. The help and user guide must state the default path,
current level mapping, size bounds, and the fact that logs can contain local
paths and process metadata even though document content is excluded.

In persistent mode, verbosity and destination are properties of host startup:

- `--serve`, an explicit session start, or the launch that creates a missing
  host passes its selected log level and destination to that host.
- An attachment to an already-running host does not change its logger. The
  host retains its current path and level until it is restarted.
- Supplying `-v` or `--log` while attaching to an existing host must not be
  silently presented as if it reconfigured that host. The command reports that
  the existing host retained its logging configuration and that a restart is
  required to change it.
- No runtime log-level mutation command or protocol message is required. This
  keeps logging out of the local protocol and avoids a second configuration
  lifecycle.

`:service-health` includes the owner role, active log level, resolved log path,
and any logger initialization or write failure. In persistent mode these are
host facts. A newly attached client therefore sees how the host that owns its
workspace is actually logging rather than the flags of the client process.

### Records and context

Each record is a single human-readable line with an RFC 3339 timestamp, level,
subsystem or target, process role (`standalone` or `host`), PID, and message.
Structured key-value context may be appended where it materially disambiguates
an event. The useful stable identifiers are workspace ID, service name,
language-server generation, request ID, buffer ID and revision, terminal
session ID, and transport connection role. Source text is not useful identity.

Initial instrumentation is limited to diagnostic boundaries:

- process startup, version, role, workspace identity, and orderly shutdown;
- persistent-host publication, retirement, restart, and forced termination;
- client attachment, detachment, incompatibility, connection closure, stalled
  writes, malformed or truncated frames, and shutdown flushing;
- optional-service startup, readiness, stop, restart, and failure, including
  language servers and filesystem watchers;
- PTY child exit and unexpected terminal-session failure;
- Git or helper execution failures already converted into typed results;
- background task termination when it would otherwise silently remove a
  service; and
- panics.

Routine keystrokes, rendered frames, successful editor commands, buffer edits,
and complete LSP request or response bodies are not logged. A failure is
recorded once at the boundary that has the most context; every `Result`
propagation layer must not emit a duplicate copy of the same error chain.
Existing status, notification, and subprocess-detail behavior remains intact.
An actionable failure still reaches the person using the editor even when the
logger is disabled or unavailable.

Install a panic hook that makes a best-effort final record containing the
thread, panic location, message, and backtrace when backtraces are enabled.
The normal panic output remains available in foreground and standalone modes.
The hook matters especially for a detached host, where stderr is not a durable
diagnostic destination. Logging must not change unwind behavior or terminal
restoration.

### Privacy and safety

Default and verbose logging must never contain buffer text, selections,
clipboard contents, typed or pasted text, terminal contents, credentials,
environment-variable values, unrestricted subprocess output, or full LSP JSON
messages. Paths and executable names may appear where they are already needed
to identify a failing local operation, so documentation must tell people to
review a log before sharing it. Workspace identity should prefer the existing
stable workspace ID over repeating the absolute project path on every record.

The file sink is bounded and cannot block input, rendering, local-protocol
handling, or service event draining. Use a bounded queue and a single writer
owned by the logging process, or an equivalently small non-blocking design.
When overloaded, diagnostic logging is best effort; dropped-record counts may
be summarized later, but producers must not wait for disk. Shutdown and panic
paths make a bounded best-effort flush and never wait indefinitely.

Keep at most 4 MiB in the active file and one 4 MiB previous file. Rotation is
owned by the same process as the file, happens both across long-running hosts
and at startup when necessary, and never lets an old host log grow without
bound. These limits are centralized constants and operate on bytes rather than
assuming valid character boundaries in arbitrary error text.

Failure to create or write the default log degrades logging rather than
preventing editing or preventing a persistent host from serving. The failure
is shown through stderr when available, retained as a notification once an
`App` exists, and visible in `:service-health`. Failure to honor an explicit
`--log` path is a startup error because silently choosing another destination
would make the requested diagnostic capture misleading.

The implementation may use a small logging facade or an existing Rust logging
crate, but the dependency is not part of the application architecture. Do not
introduce a logging service into the local protocol, serialize domain objects
for convenience, or expose logging types through editor, snapshot, workspace,
LSP, Git, or terminal APIs. Instrumentation receives compact values at the
existing ownership boundaries.

### Coverage

Tests must exercise behavior rather than merely assert that logging calls
compile:

- A standalone process writes warning and error records at the default level
  and omits info, debug, and trace records.
- Repeated `-v` selections enable the documented levels and cap at trace.
- A newly started persistent host owns `host.log`; records continue while its
  client is detached, and a later client can open the same log.
- Attaching with different logging flags does not change an existing host's
  path or level and reports the restart requirement.
- Client-side failures do not append to `host.log`, while attachment and
  disconnection facts observed by the host do.
- Concurrent standalone instances do not share a writable log or rotation
  owner.
- Rotation enforces the active and previous file bounds, including across a
  host restart.
- An unavailable default destination leaves editing operational and appears in
  service health; an invalid explicit destination fails startup clearly.
- Queue saturation or a stalled writer cannot stall an editor or host event
  producer.
- A subprocess panic test verifies that a host panic leaves its location and
  message in the host log without preventing the normal process failure.
- Redaction tests use representative input events, buffer text, clipboard
  values, terminal output, LSP payloads, and environment values and verify that
  none reaches a record.

Update `README.md` and `docs/user-guide.md` with the troubleshooting workflow,
startup controls, ownership rules, bounds, and privacy warning. Because the
change adds a command and a generated read-only buffer, also update the command
inventory, help/manual text, `context/reference/helix-keymap-v1.md` where the
deliberate Helix similarity is recorded, and
`context/reference/ui-vocabulary.md` for the surface and snapshots.
