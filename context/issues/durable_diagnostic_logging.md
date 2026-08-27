# Durable diagnostic logging with persistent-host ownership

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

## Expected behavior

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

## Levels and startup controls

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

## Records and context

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

## Privacy and safety

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

## Coverage

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
