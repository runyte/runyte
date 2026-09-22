# WP5: native parent-terminal authorization and routing

Read-only design against the current Windows branch. No source edits, builds,
tests or native probes were performed. This package supplies parent authority
and routing; it does not implement the interactive frontend or process watcher.

## Existing seams

- `src/workspace/parent.rs`: Unix-only host seed, terminal-specific capability,
  bounded `RUNYTE_PARENT_CONTEXT` parser and redacted `ParentLaunch` debug output.
  Its marker currently names a ready-record path; the capability is not sufficient
  authority by itself.
- `src/terminal/mod.rs:2543`: Unix `TerminalSessions::validates_parent` combines
  capability, live terminal and socket peer session membership. Windows currently
  passes no parent context from `open`.
- `src/terminal/pty_windows.rs:295`: `spawn_in_context` ignores its context.
  `Pty` already retains the exact process and per-terminal job; the child is
  created suspended, assigned to that job, then resumed. Close removes the
  session and starts owned asynchronous job/ConPTY teardown.
- `src/terminal/windows_command.rs::environment`: strips inherited parent marker
  names case-insensitively and installs `standalone`. This is the injection point
  for an explicitly supplied native context, with no global environment mutation.
- `src/windows_host/clients.rs`: each accepted peer retains `Arc<PinnedProcess>`;
  `_proof` is available to become the actual authorization input. It currently
  refuses interactive and parent requests, and owns wait tokens per connection.
- `src/main.rs:2219` and `run_parent_request`: Unix ParentAttach/ParentWait relay,
  active-attachment checks, one outstanding switch, receipt completion and expiry.
  `ParentSwitchWorkspace` is handled by the outer frontend; the Unix child does
  not create a nested TUI. `prepare_switch_target` runs in that outer frontend.
- `WorkspaceHost::create_parent_wait_request`: existing visible-terminal,
  overlay, buffer-lease and origin restoration rules remain shared.

## Minimal native authority

Keep Unix behavior unchanged. Add a Windows parent implementation or small
platform branch behind the common parent facade; do not spread native handles
through protocol DTOs. Suggested narrow types:

1. `ParentLaunch`: owns a BCrypt-generated 32-byte host secret and a validated
   captured endpoint identity. Derive each terminal capability from that seed
   and the never-reused terminal ID, as Unix does. Replacing the host seed revokes
   old markers. Debug output must omit secret and derived capabilities.
2. Native `ParentContext`: bounded version, terminal ID, capability and captured
   `EndpointMetadata` (or the equivalent exact pipe/process/incarnation/project
   fields). Capturing native metadata avoids following a later ready-file
   replacement just to find the parent. Treat every environment field as a
   candidate hint, validate all bounds, then authenticate the actual server
   through `windows_lifecycle::connect_control`. Retain that connection's peer
   proof for the request lifetime. A metadata PID alone authorizes nothing.
   The private environment marker can be platform-specific without changing the
   existing ParentAttach/ParentWait protocol messages. Redact its Debug output.
3. `Pty::contains_peer(BorrowedHandle<'_>) -> io::Result<bool>`: query
   `IsProcessInJob(peer, self.job, ...)` using the owned exact terminal job.
   Expose a membership operation, not a clonable job handle or mutation API.
4. Native `TerminalSessions::validates_parent(id, capability, &PinnedProcess)
   -> io::Result<bool>`: require matching launch capability, present live terminal,
   non-stopping PTY, an unsignaled retained peer, and exact job membership.
   Check the retained leader/control state rather than relying solely on a
   possibly queued terminal-exit event. Missing/closed terminals, query errors,
   exited peers and foreign jobs fail closed. No PID reopening or process-tree
   ancestry walk belongs in this method.

The current pinned peer's limited query rights suffice for IsProcessInJob; the
job query uses the already owned handle. Microsoft documents both the required
rights and the distinction between testing a specific job and any job.
[IsProcessInJob](https://learn.microsoft.com/en-us/windows/win32/api/jobapi/nf-jobapi-isprocessinjob)

Keep `windows_process::new_job` and ConPTY limits unchanged: kill on close, no
breakaway. Nested-job descendants remain in the ancestor chain, and an immediate
job which disallows breakaway prevents escape even if ancestors allow it.
[Nested jobs](https://learn.microsoft.com/en-us/windows/win32/procthread/nested-jobs)

Install the host's ParentLaunch before opening any persistent terminal, from its
prepared/bound endpoint identity. A standalone terminal always receives the
explicit standalone marker. Existing inherited markers must still be removed
case-insensitively. Thread the explicit optional marker into the native environment
builder; preserve sorted UTF-16 environment handling and ordinary shell selection.
Do not reuse Unix shlex for Windows EDITOR/VISUAL parsing. Bare Runyte-to-`--wait`
rewriting needs a separately tested native single-command recognition boundary;
custom commands remain untouched.

## Routing and lifecycle

Parse parent context early for explicit persistent/wait launch modes, before App,
terminal acquisition, ordinary detached startup or fallback. If supplied context
is stale, incompatible or standalone, return the specific refusal; do not fall
back to spawning from inside the restricted terminal job.

On the host, use the actual retained connection proof. Revalidate capability and
terminal membership when the queued request is actually handled, including after
any rename deferral. Prove live native peer/job membership before treating the
supplied capability as authority; a copied token never substitutes for membership.
This also excludes a launcher created outside the terminal job through an
external opener/ShellExecute helper, even if its marker was inherited. Couple
authorization to the current interactive attachment generation.
ParentWait keeps the existing visible-origin and overlay checks, creates its
wait through WorkspaceHost, and puts the token in that requesting peer's wait
set. Disconnect, terminal close and explicit cancellation retain the existing
wait cleanup and origin-restoration semantics. No control request becomes native
physical input or bypasses provider-write approval.

ParentAttach additionally requires the active terminal, ready parent UI, the
same active attachment generation and no other handoff. Use one bounded pending
handoff record containing child connection/proof, terminal ID, frontend
connection/proof, unpredictable receipt, captured target request and deadline.
Bind completion to the same retained frontend process, not just receipt text
received on any later control connection. A reconnect can prove the same process
through its newly authenticated peer handle. Receipt replay, wrong peer, detach,
origin close and expiration return one terminal result and release the record.

Destination preparation must run outside the requesting ConPTY job. The preferred
native coordinator has the authenticated parent host resolve/prepare the target
using its captured configuration and DiscoveryScope, then uses existing native
startup to obtain actual ready-peer proof. Its provisional child remains owned
through cancellation. The future outer frontend receives the switch only after
the target is prepared, and acknowledges only after successful attachment.
Alternatively, preserving the Unix outer-frontend preparation is viable once
native frontend ownership proves that executor is outside the terminal job;
that dependency should be explicit, not a child-side breakaway fallback.

The host already owns its terminal jobs and is outside those child jobs. Existing
startup release/breakaway rules apply only to startup's private job; never relax
ConPTY containment to make this path work. External ancestor restrictions remain
observable native startup errors. Keep destination startup work off the host
event loop, admission bounded, and retain the routing owner until cancellation
and child cleanup finish. This design does not add a new process-watch mechanism.

The current internal native host has no interactive attachment. It can accept
the authority/plumbing implementation and tests, but must continue to refuse
ParentAttach/ParentWait until the native frontend supplies attachment-generation
and origin ownership. This is an existing integration dependency, not a new
architecture blocker.

## Bounded implementation and acceptance sequence

1. Native marker/seed and explicit environment propagation, plus retained-handle
   job-membership API. Preserve ordinary ConPTY creation/teardown and Unix tests.
2. Host-side authorization and bounded routing state machine with injected
   frontend/startup actions. No public interactive availability yet.
3. Wire early launcher routing and parent wait/switch handling when the actual
   native frontend contract is ready; keep protocol and wait ownership shared.

Required fixtures use compiled helpers and explicit temporary config/cache/state;
the outer test retains job ownership for the whole tree on panic/cancellation.
Use acknowledgments and existing cleanup completion events, not sleeps.

- A compiled ConPTY descendant sends a real native control request and succeeds
  only with that terminal's capability. Include a further descendant/nested job.
- Copy the marker to another terminal job and to an outside process: both fail
  despite valid text. Wrong terminal/capability, replaced host identity, exited
  retained peer, closed terminal and query failure cause no editor/startup effect.
  The outside-process case represents an external-opener helper without relying
  on personal file associations or launching an interactive shell application.
- Verify auth is revoked immediately on session removal while asynchronous PTY
  teardown is still outstanding; verify PID values are never reopened for proof.
- Check marker replacement case-insensitively, standalone behavior, malformed/
  oversized input, native non-ASCII path identity and redacted debug/error output.
- Delay a queued parent request behind rename, then change attachment/close its
  terminal: dequeue must reject it. Cover busy overlays and invisible origins.
- Cover ParentWait token ownership, wrong-peer cancellation, disconnect cleanup
  and restored terminal input ownership after completion/refusal.
- Cover duplicate switch, receipt replay/wrong frontend, source disconnect,
  timeout, failed destination startup, and successful exact target attachment.
- In controlled nested-job containment, prove destination startup executes from
  the parent-side executor and survives closing only the requesting terminal;
  also prove the terminal launcher itself cannot escape its unchanged job.

No user decision is needed for the authority boundary. The integration choice
is which already-outside owner prepares the destination; prefer the parent host
worker, while retaining the existing outer frontend receipt/attachment protocol.
