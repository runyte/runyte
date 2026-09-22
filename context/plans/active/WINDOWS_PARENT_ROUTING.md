# WP5: native parent-terminal authorization and routing

Retained architecture for native parent authority and routing. ParentWait and
the private ParentAttach handoff are implemented behind closed public launch
gates; the remaining work is the separately reviewed public routing package.

## Existing seams

- `src/workspace/parent.rs` now owns the platform-specific parent marker and the
  private Windows `run_wait` and `run_attach` clients. Both authenticate one
  captured endpoint and retain natural-parent supervision; public launch routing
  remains separate.
- `src/terminal/mod.rs`, `pty_windows.rs` and `windows_command.rs` install the
  explicit marker, retain the terminal job and validate the requesting process
  by handle and exact job membership. Inherited markers are still removed
  case-insensitively.
- `src/windows_host/clients.rs` binds parent requests to the live control peer,
  ready interactive attachment generation, visible terminal and capability. It
  owns wait tokens and the bounded ParentAttach response ordering.
- `src/workspace/windows_service.rs` resolves each ParentAttach against a fresh
  catalog and starts an exact missing destination with frozen inputs outside the
  ConPTY job. Request cancellation remains service-owned through provisional
  startup cleanup and never retires an authenticated existing winner.
- `src/tui/windows_frontend.rs` performs the prepared destination attachment and
  original-source confirmation. The parent-only commit acknowledgment is the
  readiness signal; the frontend's same-connection confirmation is the
  irreversible handoff point. A lost final source receipt cannot discard the
  authenticated destination.
- `WorkspaceHost::create_parent_wait_request` continues to provide the shared
  visible-terminal, overlay, buffer-lease and origin restoration rules.

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
Bind confirmation to the same retained frontend process and original source
connection, not just receipt text received on any later control connection. A
reconnect cannot inherit its reservation. After that connection confirms the
nonfinal commit acknowledgment, its expected close may leave the already
authenticated frontend process proof as owner through the atomic destination
decision. Receipt replay, wrong peer, detach, origin close and expiration before
that decision return one terminal result and release the record.

The parent path reuses `NativeSwitchPrepared`, but commit acceptance is not its
terminal result. The source host first sends a parent-only nonfinal commit
acknowledgment on the original interactive connection and retains both the
source reservation and child. The frontend confirms observation on that same
connection. Only then may the host answer `ParentAttached`, release the source
reservation and send the ordinary final committed receipt. Ordinary native
switches retain their existing two-phase behavior.

Destination preparation must run outside the requesting ConPTY job. The preferred
native coordinator has the authenticated parent host resolve/prepare the target
using its captured configuration and DiscoveryScope, then uses existing native
startup to obtain actual ready-peer proof. Its provisional child remains owned
through cancellation. Cancellation interrupts readiness immediately and spends
one cleanup budget proving job emptiness and removing only the provisional
publication. Destination release is a bounded pending host-loop state rather
than an inline await; input, transport and terminal events continue while it is
settled. Release failure retains the armed owner through the same job and
publication cleanup. The service first yields a still-armed commit decision;
the host revalidates the exact child, retained frontend proof, terminal authority
and deadline before making that single irreversible decision, then retains the
result receiver until the worker reports released or settled. A successful
release can therefore produce `ParentAttached` without a contradictory later
authority check.
Expected source-connection closure after same-connection confirmation keeps the
already retained frontend process proof rather than undoing the commit. The
future outer frontend receives the switch only after the
target is prepared, and acknowledges only after successful attachment.
Alternatively, preserving the Unix outer-frontend preparation is viable once
native frontend ownership proves that executor is outside the terminal job;
that dependency should be explicit, not a child-side breakaway fallback.

The host already owns its terminal jobs and is outside those child jobs. Existing
startup release/breakaway rules apply only to startup's private job; never relax
ConPTY containment to make this path work. External ancestor restrictions remain
observable native startup errors. Keep destination startup work off the host
event loop, admission bounded, and retain the routing owner until cancellation
and child cleanup finish. This design does not add a new process-watch mechanism.

The private native host/frontend pair now supplies attachment-generation and
drawn-frame readiness for ParentWait and ParentAttach. Public persistent-session,
`--wait` and parent-attach launch routing remains gated until its own acceptance
package opens those routes.

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
