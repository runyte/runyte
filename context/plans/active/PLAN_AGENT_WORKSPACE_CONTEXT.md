# Agent access to live workspace and terminal context

## Status and outcome

Active, 2026-09-17, planned against source `178814a`. Implementation and
subagent reviews are authorized. Release publication remains a separate action.
Extends
[the open issue](../../issues/agent_workspace_context.md). Cross-workspace
reading is part of the intended delivery, not an optional future broker.

A person can run Claude and Codex in two Runyte terminal panes and ask either
to read the other's displayed output. The same interface reaches terminals,
unsaved buffers, selections, and panes in other authorized live workspaces,
without attaching to them or copying text through the clipboard. Agents pull
context when asked; they do not continuously receive other agents' output.

Agents may also edit buffers, including inserting newlines, under a separate
grant. Terminal text is proposed for individual native approval in an overlay;
no proposal writes to the PTY before approval. Approval inserts text without
Enter, preserving the explicit no-submission boundary. Approval to execute is not assumed from approval to insert.

The recommended design is an external MCP bridge with scoped connections to
Runyte hosts. Runyte owns discovery, authorization, bounded reads, and resource
identity. The bridge owns agent tools and protocol adaptation and is released
separately. No agent-specific API, conversation parser, or MCP dependency belongs
in the editor core.

## Implementation progress

The approved plan is committed as `80eceb5`. The first implementation slice
adds internal terminal primitives; the external feature is not yet available:

- `src/terminal/read.rs` captures bounded, owned screen/tail text, with separate
  row, UTF-8 byte and visited-cell limits. Returned rows identify their source
  and clipping, and preserve Unicode base/combining sequences and blank rows.
- A dedicated terminal read revision covers output, resize, exit and shared
  history eviction without invalidation from native review or scrolling.
  History-loss counts describe the active grid rather than review eviction.
- `src/terminal/proposal.rs` validates bounded, single-line literal text and
  rejects submit/control characters and injected escape framing. It provides
  no send operation and grants no approval or input authority.
- Eighteen tests in `src/terminal/tests/read.rs` and `proposal.rs` exercise
  these primitives without processes, sockets, configuration or runtime files.

Shared external ownership, authentication, capability admission, native grants,
discovery, the bridge, buffer edit admission, proposal overlays and cancellable
PTY delivery remain to be implemented. No external protocol has changed in
this slice. Freeze the admission contract before exposing these primitives.

### Subagent review comments and disposition

Two independent reviews examined the design and first implementation slice.
The retained comments below are technical findings, not unrestricted transcripts.

- **PTY acceptance is not delivery:** `Pty::write` currently acknowledges queue
  admission; its writer does not implement cancellable proposal ownership or
  completion acknowledgments. Keep validated text disconnected from sending in
  this slice. The proposal delivery stage must add both before reporting
  `delivered` or promising revocation of queued input.
- **Read revision gaps:** feed-only revisions miss resize and shared-history
  eviction. Added a dedicated revision and tests for both, including history
  evicted because another terminal produces output.
- **History-loss mismatch:** native `history_truncated` can mean review-only
  eviction and does not track normal capacity eviction. Derive active-grid
  loss from retired rows minus retained history, with tests for both eviction
  paths, explicit scrollback clear, alternate screen, and emulator reset.
- **Whole-row decoding is insufficiently bounded:** decode cells directly,
  including blank and continuation cells in the work budget. Preserve retained
  combining marks and clip whole base/combining sequences. Review suggested a
  space-plus-combining fixture, but the emulator currently discards those marks
  before decoding; this slice preserves that existing emulation behavior. The
  decoder trims only plain blanks. A wide-cell boundary fix ensures an exhausted
  cell budget never represents half a glyph.
- **Weak truncation assertion:** the final review caught an OR condition that
  could accept incorrect byte/cell flags. Replaced it with checks that both
  flags are false in the row-limit-only case.
- **Allocation accounting:** future retained-snapshot quotas must include
  String/Vec capacity and row metadata, not only decoded text lengths.
- **Invisible Unicode:** proposal text may contain bidi or zero-width Unicode;
  the future approval overlay must display it explicitly. Validation alone
  does not establish display safety or prove that arbitrary PTYs have no
  reaction to ordinary characters.
- **Compatibility and authority:** keep frozen protocol fixtures unchanged.
  Internal reads and validation do not establish external permissions, native
  approval or PTY delivery; those stages need their own independent reviews.

The implementation review found no remaining production correctness defect in
this slice after the wide-cell fix.

### First-slice validation — Linux, 2026-09-17

Validated on `x86_64-unknown-linux-gnu` with Rust 1.97.1 and cargo-llvm-cov 0.9.0:

- `cargo fmt --check` passed.
- `cargo clippy --all-targets -- -D warnings` passed.
- `cargo test` passed: 3,672 tests, no failures, 34 existing ignored tests.
- `cargo llvm-cov --locked --workspace --summary-only --fail-under-lines 89`
  passed: 121,179 of 131,908 lines covered (**91.87%**). The 89% floor is unchanged.
- Local Markdown links and `git diff --check` passed.

The complete suites ran with the local socket and PTY access required by the
existing fixtures. The new tests themselves need neither. No native macOS
result is claimed; CI must validate that first-class target before merging.
No startup/idle performance result or end-to-end agent workflow is claimed for
these internal primitives.

## Scope and user experience

1. Install the bridge and configure its stdio MCP entry in each agent. Exact
   client configuration is verified against the clients' documentation during
   bridge implementation, rather than embedded in the editor contract.
2. In each workspace to be shared, use a native context-access command to pair
   the bridge and grant terminal reads, editor reads, buffer edits, or terminal
   proposals independently. Session-only approval is the default; remembering
   a grant is explicit. Proposal permission never pre-approves terminal input.
3. Name terminals with the existing terminal rename workflow, for example
   `Claude` and `Codex`. Names help discovery but are not identities and do not
   prove which program is running.
4. Ask an agent to list accessible workspaces and terminals, read `Claude` in
   the current workspace, or read the build terminal in another workspace.
   Tools return explicit handles and source labels, so duplicate names do not
   silently select a target.
5. Inspect connected readers and recent read metadata, or revoke access,
   through the same native context-access surface.
6. Review each proposed command or prompt in a native overlay naming its agent,
   workspace and terminal. **Insert text** inserts that exact text once, without
   Enter; **Reject** writes nothing. Submit from the terminal personally.

"All open sessions" means all authorized, running persistent-session hosts in
the current Runyte environment, including detached hosts. It includes live,
hidden, and exited-but-retained terminal sessions. It excludes stopped hosts,
destroyed terminals, and output already evicted by existing retention limits.
Cross-environment discovery is an explicit option corresponding to
`--include-hidden`, with independent grants for those targets.

Standalone workspaces also need a context listener when explicitly enabled.
They have a distinct live registration that disappears on exit; discovering
one must not create a persistent session or add it to persistent-session
navigation. Deliver this after persistent multi-workspace access, before
claiming support for every open Runyte workspace.

Reading another agent's terminal is observing its rendered output, not accessing
its private conversation database or reasoning. The bridge cannot send arbitrary
keys, submit prompts, interrupt processes, focus panes, or start/stop sessions.
Its only terminal input path is individually approved text insertion. An
automatic conversation relay remains outside this work.

## Current implementation and gaps

- `src/plugin/application.rs` defines the stable `runyte-1` application
  protocol and sixteen capabilities. `terminals` permits `terminal.open`;
  it grants neither reading nor input. `workspace` exposes buffers and panes.
- `src/workspace/host/plugin_editor.rs` implements revision-bound text reads,
  immutable buffer snapshots, and selections. Its `text` capability also
  permits editing, and `selections` permits selection changes. Granting those
  capabilities to an external reader would be too broad. Buffer edit permission
  must be explicit and independent of read permission.
- `src/plugin/editor.rs::Pane` identifies a buffer, or null for a terminal.
  There is no terminal handle, focused-pane inventory, or native viewport read.
- `src/terminal/mod.rs` already exposes `plain_line_count`, `plain_line`, and
  `plain_line_with_id`, using decoded emulator state without copying all output.
  Drawing revision, content revision, retained-line identity, and completed-line
  activity are distinct. None alone currently specifies external read semantics.
- Terminal history is bounded to 5,000 rows per terminal and a shared 64 MiB
  retained-cell/review payload budget. Alternate screens have no history.
  Review snapshots can differ from live terminal content.
- `src/workspace/catalog.rs` and `transport.rs` discover and validate live
  persistent hosts, including explicit discovery across isolated environments.
  Workspace identity alone does not distinguish two live hosts for one root.
- Applications currently enter through host-managed plugin processes. The
  bundled local-client protocol in `src/protocol/` is private, and its control
  role is not a suitable external read-only API.

These are reusable foundations, not evidence that external application
admission or terminal reads already exist.

## Architecture and ownership

```text
Claude or Codex
    | MCP over stdio, started by that agent
External context bridge
    | bounded discovery; lazy connections to explicit targets
    +-- scoped runyte-1 connection --> workspace A --> terminals / editor
    +-- scoped runyte-1 connection --> workspace B --> terminals / editor
    +-- scoped runyte-1 connection --> workspace C --> terminals / editor
```

Each agent may start its own bridge process. There is no required machine-wide
daemon. Connections are opened lazily, and no read depends on switching the
interactive TUI or on which workspace happens to be focused later.

Add an opt-in, owner-private Unix-domain context listener to each participating
workspace. It accepts a restricted external application profile of `runyte-1`,
using the public application envelopes, negotiation, errors, and read values.
Document its admission handshake as a new public transport; do not expose the
private frontend/control protocol or serialize core workspace values directly.

Extract the read request context and handle ownership from assumptions about
`app.plugins.instances[owner]`. A host-created external reader owns its own
handles, grant generation, request budget, subscriptions, and snapshots, but
does not get a plugin process, registered commands, views, jobs, or activity
leases. Existing process applications retain their current behavior. Both
adapters call the same semantic read implementation.

This extends the issue's ordinary-application bridge suggestion with external
scoped admission, read-only by default. It avoids requiring a separate listening
plugin process inside every workspace and gives the host direct control over revocation. Do
not make a bridge impersonate a configured plugin instance or borrow the
interactive client's attachment credentials.

Likely ownership:

| Area | Implementation location |
| --- | --- |
| Decoded terminal ranges and read revision | `src/terminal/`, with a small read module |
| Stable read DTOs and validation | `src/plugin/application.rs`, `editor.rs`, new terminal wire module |
| Shared read dispatch and reader resources | `src/workspace/host/`, extracted from `plugin_editor.rs` |
| Buffer transactions and terminal proposals | shared editor dispatch, new host proposal coordinator, and `src/terminal/` input queue |
| Local listener, admission and reader cleanup | new `src/workspace/context/` module |
| Exact-workspace grants | new `src/context_trust.rs`, using private-storage patterns |
| Native approval and reader inspection | new `src/app/context_access.rs`, command registry and snapshots |
| Discovery | workspace catalog/transport validation plus a narrow context inventory |
| Agent tools and MCP adapter | independently released bridge project |

## Authorization and discovery

Introduce separate read capabilities, provisionally `terminal_read` and
`editor_context_read`. The first grants terminal inventory and decoded reads;
the second grants buffer/pane inventory, selection reads, and buffer/viewport
reads. Terminal-to-pane associations are available with terminal permission
without exposing unrelated buffer names. Neither capability grants mutation.
Add `buffer_edit` for revision-checked transactions on already-open editable
buffers and `terminal_propose` for requesting native review of terminal text.
Require the corresponding read scope for discovering and inspecting targets.
The external profile explicitly allowlists these operations; it rejects other
mutations regardless of declared capabilities, including `terminal.open`,
`buffer.open`, generic key dispatch, and direct PTY input. No terminal submission
capability or configurable bypass is included.

Configuration can request these capabilities but cannot approve external
exposure. Store grants outside the project tree, following the canonical-root,
owner-private storage pattern in `lsp_trust.rs`. Bind them to the paired bridge
credential identity, exact workspace root, selected scopes, and grant version.
An approval in workspace A never authorizes access to workspace B.

Pairing creates an unguessable credential in private per-user runtime or state
storage and uses a native affirmative action to approve its scopes. Authenticate
the credential and OS peer ownership before issuing handles. Labels such as
`Codex` are descriptive, not authenticated executable identities. The grant is
for the paired local bridge identity; distinguishing two agents requires
separate credentials if their permissions should differ. Credentials never go
in command-line arguments, terminal output, repository configuration, or logs.

The native approval shows workspace, reader identity, requested scopes, and
whether the decision is remembered. It explicitly covers unsaved text and
potentially sensitive terminal output. Use the physical-input ownership pattern
from provider-overwrite approval: macros, plugin requests, and synthetic input
cannot approve it. A detached host without an existing grant returns a bounded
authorization failure; it does not wait indefinitely or steal another TUI.

Revocation increments the grant generation, invalidates handles and snapshots,
cancels pending reads and proposals, drops unsent content, and disconnects the
reader. Check authorization at admission, before publishing a response, and
before applying an edit or approving/enqueuing terminal text. Bytes already
delivered cannot be recalled. This mechanism controls Runyte's own API; it is not
an OS sandbox against hostile processes already running as the same user.

Add a bounded machine-readable discovery command, provisionally
`runyte --context-list --json`, backed by existing validated discovery. Give its
output a documented versioned schema. A record carries workspace identity,
human label, host incarnation, endpoint identity, mode, and supported context
transport version. It contains no content or credentials. Authenticate and
obtain scopes from the endpoint before listing a workspace as readable.

Include only live context-enabled endpoints. Do not start hosts or bridges,
attach, modify recent-visit order, or silently expand to the owner-wide
inventory. Distinguish duplicate roots hosted in different environments through
endpoint identity and a random host incarnation, never workspace ID alone.
Validate endpoints using existing owner, symlink, liveness, and identity rules;
do not introduce unrestricted path scanning.

Host restart, terminal close, and reader reconnect invalidate their respective
handles. MCP handles wrap the target host incarnation and connection identity.
Session renaming cannot retarget a handle. Ambiguous names return candidates.

## Read contract

Names below are proposed API names, not existing operations.

| Operation | Contract |
| --- | --- |
| `terminal.list` | Paginated retained terminal handles, names, running/exited state, visible pane association, dimensions, screen kind, and read revision |
| `terminal.read` | Explicit terminal, `screen` or `tail`, maximum rows and bytes; optionally require a revision |
| `terminal.snapshot.open/read/close` | Capture an explicitly bounded region once and page it immutably while the child continues producing output |
| `pane.context.list` | Explicit focused pane and buffer/terminal identities, with inventory/attachment revision |
| `pane.viewport.read` | Bounded semantic rows from one native pane, plus geometry, source revision, attachment generation and projection metadata |
| Existing buffer/selection reads | Reuse scalar ranges, revisions and immutable snapshots, admitted under the new editor read scope |

Capture a live terminal read in one host turn and return its revision with its
text. A supplied stale revision fails rather than reading a different state.
The initial screen/tail read need not supply a revision: this prevents a busy
terminal from starving a caller between inventory and read. Paging uses a
snapshot, never repeated mutable row offsets masquerading as one document.

Define a terminal read revision covering all observable text/geometry changes:
output, carriage-return rewrites, erase/reset, screen switch, resize, and history
eviction. Keep metadata lifecycle revisions separate where appropriate. Audit
all mutation sites; the existing feed-only content counter and drawing revision
must not be assumed sufficient. Line IDs locate retained rows but do not prove
their content is unchanged.

`screen` means the current emulator screen; `tail` means the newest bounded
region of the active grid's retained history followed by its screen. Both are
independent of native scroll position or a frozen review. Alternate-screen
reads return only its current screen and explicitly report no history; do not
silently splice in primary-screen output. `pane.viewport.read` instead reports
what the pane displays, including a captured review and its older revision.

Return decoded Unicode presentation rows, preserving row boundaries and blank
rows. Omit ANSI/OSC sequences, styles and wide-cell continuation placeholders;
retain the emulator's supported combining marks. Document trailing-blank
normalization. Physical rows are not shell commands or agent messages, and
wrapped rows are not reconstructed into invented logical paragraphs.

Every result identifies the workspace, host incarnation, resource handle,
capture revision and screen kind. It states returned bounds, available retained
rows, older-history loss, and truncation reason. An empty result is distinct
from denied, closed, stale, unsupported, or unavailable. Preserve provenance
when several results are combined; there is no atomic cross-workspace snapshot.

Initial proposed limits, finalized and advertised before contract freeze:

- Default terminal tail: 200 rows and 64 KiB of UTF-8 text.
- Maximum one read: 1,000 rows and 256 KiB of decoded text, with a separate
  serialized frame ceiling accounting for JSON escaping and metadata.
- Capture at most 4 MiB per terminal snapshot; at most two snapshots and
  8 MiB retained terminal snapshot text per reader; expire after 30 idle seconds.
- Admit at most eight external readers per workspace and sixteen outstanding
  requests per reader, with a host-wide retained-byte and fair-work budget.
- One bridge holds at most eight active workspace connections, opened lazily;
  it can list further targets and release an idle connection to reach one.

Bound cells visited as well as bytes returned. Decode only the requested range,
never call full-scrollback `plain_text()` and truncate afterward. Very wide rows
may need a scalar-safe partial-row result with explicit clipping metadata.
Large snapshot capture must fail a work bound or use revision-checked bounded
steps; it must not monopolize the editor loop. Global allocation accounting
must include snapshots across all readers and existing terminal review payloads.

For native viewports, reuse prepared semantic snapshot logic without changing
focus, scroll, terminal size, review state, unread/bell acknowledgment, or buffer
offsets. Keep content padding and generated presentation hints distinct from
source text; expose scalar mappings when available. A detached workspace can
serve buffers and live terminal content, but viewport reads return
`no_frontend` rather than inventing a current display from old dimensions.

## Buffer editing

With `buffer_edit`, reuse `buffer.edit` with an explicit buffer handle, expected
revision, scalar ranges, and bounded replacement text. Newlines are ordinary
buffer text and are allowed, including multiline insertions and replacements.
Do not simulate editor keystrokes: one request is one atomic undoable transaction.
Reject stale revisions, read-only/generated projections that disallow edits,
closed targets, and changes beyond existing transaction limits. Preserve pane
focus and existing transaction semantics for adjusting selections.

An edit grant does not include saving, closing a wait-owned buffer, completing
an external-editor request, applying a directory filesystem plan, or sending a
buffer to a terminal. In particular, editing an agent's external prompt buffer
must not submit the prompt by completing its wait request. Save and lifecycle
operations need separately specified permissions and are outside this plan.

## Terminal proposals and native approval

Expose `terminal.input.propose` and a bridge tool `propose_terminal_text`.
Accept an explicit terminal handle, bounded literal text, an optional short
reason, and a client request ID. The host captures immutable text, owner,
workspace/host incarnation, terminal identity, grant generation, and expiry.
Return a proposal handle immediately; status reads report pending, rejected,
expired, cancelled, stale, queued, delivered, or outcome unknown as appropriate.
Delivery means bytes written to the PTY, not that the child accepted a command.
No externally callable approve operation exists.

The target workspace owns the overlay. It shows the requesting bridge identity,
workspace and terminal labels, exact text, and a separate untrusted reason.
Render proposals as literal text, with visible whitespace and escaped invisible
characters, never Markdown or terminal escapes. Show every byte's meaning;
scroll long proposals without approving an unseen truncated suffix. Keep the
text immutable while presented. Editing a proposal creates a new reviewed
value, never a hidden change to an already approved request.

The affirmative action is labeled **Insert text**, with **Reject** as the
initial selection. Only a fresh physical frontend action in that captured
overlay can approve. Consume the approval gesture entirely: pressing Enter on
the overlay must never forward Enter to the PTY. Macros, plugins, synthetic
input, repeated keys, and command strings cannot approve. Each approval covers
one proposal and is consumed once; no batch, remembered, or automatic approval.

Only an attached target workspace can display and accept a proposal. A detached
target returns `no_frontend` in the initial implementation; the person may
attach and request it again. Never route approval silently to another workspace
or take over a TUI. Queue at most four proposals per owner and sixteen per host,
with one overlay at a time, a 4 KiB UTF-8 text limit and a two-minute expiry.
Do not interrupt an existing prompt or confirmation. Attachment loss, grant
revocation, terminal close/exit, or owner disconnect cancels pending proposals.

Capture terminal input generation on presentation. Before insertion, revalidate
the target incarnation, grant, frontend ownership, and unchanged input generation
and terminal input modes. If native input or another approved proposal intervened,
invalidate approval and require fresh review. Show current target context during
review; terminal output is not a reliable representation of an editable command
line, and no prompt parser may declare that line empty. Insertion occurs at the
child's current input position; never clear existing input or send cursor keys.

Enforce the no-submission rule in host and terminal validation, not only the
bridge UI. Accept single-line text; reject the whole proposal for CR, LF, C0/C1
controls, DEL, Unicode line/paragraph separators, or embedded escape sequences.
This excludes Enter, Ctrl-M/Ctrl-J, Tab, interrupt/EOF keys and injected bracketed
paste terminators. Do not strip forbidden characters and insert a modified
command. Literal backslash-n remains ordinary text. Buffer edits retain their
separate multiline contract. Revalidate the decoded text immediately before
queue admission, and never append a newline.

Use a dedicated validated text path. The current general `send_text` permits
multiline paste and `keys::encode_paste` converts bare newlines to carriage
returns, so neither is an external authorization boundary. Only host-generated
bracketed-paste framing is permitted when the child requests it; otherwise send
the validated text literally. Do not offer a raw-bytes or key-sequence escape
hatch. Queue the bounded payload as one non-interleaved input item and account
for grant cancellation until writing begins. Once bytes have left, revocation
cannot undo them. Partial writes or lost acknowledgments must never trigger an
automatic retry. Deduplicate request IDs within a bounded connection lifetime;
after lost ownership, report uncertainty rather than replaying the insertion.

This guarantees that the bridge cannot emit a submit keystroke. Arbitrary
terminal programs can still act on printable characters or custom bindings;
bracketed paste is not an execution sandbox. The overlay must not claim that
insertion is side-effect-free. A guarantee that no child action occurs before
manual submission would require keeping a draft in Runyte or a cooperating
application input API; it cannot be enforced for arbitrary PTYs by filtering
Enter alone. Approval to insert must not be described as approval to execute.

## Bridge tools and observations

Start with `list_workspaces`, `list_terminals`, `read_terminal`, `list_panes`,
`read_buffer`, `read_selection`, and `read_pane`. Offer bounded snapshot paging
through the read tools. Each content tool requires an explicit workspace and
resource handle obtained from discovery. A convenience current-workspace hint
may select a discovery filter, but is neither authority nor an implicit target
for subsequent reads. Do not infer agent identity by parsing terminal titles.
Advertise `edit_buffer` only with its grant and `propose_terminal_text` only
with its grant. The latter stages text for native review and cannot approve or
submit it. Tool schemas must not offer a `submit` flag or generic key input.

Tools identify returned terminal and document text as untrusted source content,
not instructions. They return provenance and truncation as structured fields,
not text that could be confused with the terminal's own output. No tool searches
or reads every workspace's contents merely to list available targets.

Initially fetch fresh metadata on demand and avoid caching content. Later,
negotiated terminal and native-pane source observations may invalidate caches
using the existing bounded subscription machinery. They carry revisions and
lifecycle metadata only; no automatic text streaming, polling of every terminal,
append-only transcript assumption, or agent-to-agent feedback loop.

Native inspection shows active reader identities, scopes, last-read resource
and time, and revocation controls. Retain a bounded in-memory metadata ring,
not content or credentials. Routine reads update this record without producing
a notification for every request. No terminal output is persisted by this work.

## Implementation sequence and acceptance

1. **Freeze the scoped profile and admission design.** Specify capabilities,
   authenticated handshake, scope/grant lifetimes, errors, limits, identity,
   transport discovery and external-owner lifecycle. Assess additions against
   the active stable-plugin plan and compatibility register. Use negotiated
   features/new methods for changed shapes and closed event enums; preserve
   frozen stable fixtures. Add denial tests before exposing a listener.
2. **Implement terminal reads at the emulator boundary.** Add bounded range
   extraction, complete read revision semantics, and immutable bounded capture.
   Prove that reading hidden/live/reviewed terminals has no presentation or
   process side effects. Expose it through application read dispatch.
3. **Implement native grants and external read admission.** Extract shared
   read ownership, add private trust records, pairing, listener lifecycle,
   method allowlisting, cancellation, revocation and inspection.
   Reuse existing buffer reads without granting editing authority.
4. **Implement persistent cross-workspace discovery and routing.** Add the
   narrow discovery CLI, incarnation-bound targets, explicit hidden-environment
   enumeration, per-host deadlines, and fair quotas. One hung host must not
   prevent other targets from being listed or read.
5. **Deliver the external MCP bridge and terminal milestone.** Two agents in
   one workspace can discover and read each other's terminals. Then the same
   agents read an approved terminal in a second detached workspace. Access
   remains usable after TUI switching and is denied after grant revocation.
   Ship installation/configuration examples and a network-free fake MCP client.
6. **Complete editor context.** Add focused-pane metadata and native viewport
   reads; expose existing exact unsaved-buffer and selection reads through
   bridge tools. Add separately granted revision-checked buffer edits, including
   multiline text without save or external-editor completion. Add invalidation
   observations only if cache use warrants them.
7. **Deliver terminal proposals.** Add the bounded proposal lifecycle, native
   overlay, one-shot physical approval, and validated text queue. Demonstrate
   zero PTY input before approval, exact insertion after approval, and no Enter
   reaching the terminal, including when Enter approves the overlay. Keep
   rejected and expired proposals inert and require manual terminal submission.
8. **Complete standalone discovery.** Reuse the same read/grant implementation
   with an opt-in listener and ephemeral inventory, preserving standalone
   process lifetime and persistent-session navigation semantics.
9. **Validate and document.** Update the user guide, application contract,
   current schema/SDK, terminal compatibility record, UI vocabulary and command
   registry documentation. Make a fix commit, then a separate issue-resolution
   commit with its hash, following the repository issue lifecycle. Do not mark
   the issue resolved at the terminal-only milestone.

Required behavior coverage:

- Terminal tests: primary history, alternate screen, top-anchored inline TUIs,
  partial-line and carriage-return updates, clear/reset, resize, wide/combining
  Unicode, blank rows, retention eviction, exited terminals, byte/cell bounds,
  and immutable capture during output. Verify reads leave review, focus,
  unread/bell state, input, and process lifetime unchanged.
- Host tests: every ungranted or non-allowlisted mutation rejected, handle ownership and stale targets,
  absent/expired/revoked grants, queued-reply revocation, physical approval,
  snapshot cleanup, malformed frames, slow readers, and fair work under load.
- Integration tests: duplicate names/roots, independent per-workspace grants,
  detached hosts, host restart, disconnect/reconnect, stale registrations,
  hidden environments, standalone exit, and mixed supported/older hosts.
- Editor tests: unsaved Unicode buffer ranges and selections, focus races,
  pane close/retarget, scrolled terminal review versus live output, soft wrap,
  generated alignment, clipping, and detached viewport refusal. Verify multiline
  edits and atomic undo, stale-edit rejection, and no save, filesystem action,
  focus change, or external-editor wait completion from a buffer edit.
- Proposal tests: no bytes before approval or after rejection; one-shot physical
  approval; Enter used to approve never reaching the PTY; literal CR/LF,
  Ctrl-M/Ctrl-J, escape and paste-terminator injection rejected; bracketed and
  unbracketed paths; control/Unicode separators and misleading display text;
  input-generation/mode races, stale targets, detached hosts, prompt ownership,
  queued cancellation, queue limits, expiry, duplicate requests, and uncertain
  partial delivery without replay. A real-PTY fixture records inserted bytes
  and proves the command runs only after a separate native submit action.
- Bridge tests: two independent clients, explicit cross-workspace routing,
  bounded paging, source attribution, truncation and partial availability;
  no implicit writes or automatic transcript exchange. Manually confirm the
  two-agent workflow without using network/account workflows as a CI gate.

Use existing real-PTY fixtures and checked-in `src/fixtures/stand-in`, not
temporary executable scripts. Isolate all subprocess XDG configuration, runtime,
cache, trust and inventory storage in temporary fixtures; do not publish test
readers into the person's account inventory. Recompute workspace IDs from
sanitized fixture roots in any retained documentation.

For Rust implementation, run `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, and canonical
`cargo llvm-cov --locked --workspace`; retain the current 89% line floor on
Linux and macOS. Run stable-plugin compatibility/conformance checks as well.
Measure startup and idle with access disabled, enabled without readers, and
under concurrent noisy-terminal reads using `benchmarks/`. Disabled access
must add no listener, discovery scan, timer, or terminal copying; quiescent
enabled access must not poll terminal contents. Record results in the existing
performance register before claiming the editor's responsiveness is preserved.
