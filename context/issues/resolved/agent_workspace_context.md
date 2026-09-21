---
title: "External agents cannot read live workspace and terminal context"
status: resolved
reported: 2026-09-17
resolved: 2026-09-18
commit: 4d3a442
---

## Resolution

`4d3a442` — `Add scoped agent workspace context and approved terminal text`
completes the implementation planned in `80eceb5`, including the terminal read
primitives introduced by `0999b82`.

The existing application protocol could read buffer ranges but had no external
reader admission, terminal-content operations, or native viewport contract.
`src/workspace/context/` now owns a strict `runyte.context.v1` profile, private
identity/grant storage, bounded discovery and local transport. A separate
external profile keeps arbitrary plugin commands, attachment, process control
and terminal input outside the reader's authority. Each canonical workspace
independently grants read, buffer-edit and terminal-proposal scopes; revocation
cancels connections, handles, snapshots and unclaimed input. Discovery probes
exact host incarnations without starting or attaching to editors or visiting
project filesystems. Standalone editors publish ephemeral endpoints; persistent
hosts remain readable while detached.

`src/workspace/host/context_reads.rs` routes explicit owner-bound resource
handles to unsaved scalar-range reads, selections, cached native viewports and
bounded emulator output. Native terminal panes require terminal-read permission
even when reached through editor-context operations. Pane binding generations
prevent handles from becoming valid again after retargeting away and back.
Snapshots charge allocated storage against reader/host budgets; immutable
buffer ranges use sparse scalar indices so late reads do not scan the prefix.
Detached hosts expose content but refuse native viewport claims. Buffer edits
check the expected revision and apply one undoable transaction, including
newlines, without saving, changing focus or completing external-editor waits.

`src/app/context_access.rs` owns grants and terminal proposal review. Every
proposal starts at Reject and displays its captured target and literal text;
only fresh physical frontend input can approve after every page was actually
presented. Bundled protocol 53 carries the rendered frame identity so dropped
frames cannot count as review. Validation rejects CR/LF, control characters,
escape/paste injection and Unicode line separators. The PTY writer accepts one
cancellable, noninterleaved text item and reports delivery only after all bytes
are written. The approval Enter is consumed by the overlay; submitting remains
a separate native action. Queued work cancels on revocation, expiry, exit or
changed input state; uncertain started delivery is never replayed.

The separately versioned Python MCP bridge in `bridges/runyte-context/` supplies
explicit workspace/resource tools, provenance, bounded connections and client
setup examples. It uses on-demand reads without a content cache, so no new
invalidation subscriptions are needed. Native permission inspection shows
bounded identity/method/resource metadata, never captured content. The completed
[plan and review dispositions](../../plans/completed/PLAN_AGENT_WORKSPACE_CONTEXT.md)
record all nine steps and the review findings, including the native parser
omission caught by the final real-editor workflow.

Release validation on macOS exposed a race before the identity lock was
acquired: concurrent `Directory::append` calls used nonexclusive `O_CREAT`,
and `openat` could return `ENOENT` while another caller created `identity.lock`.
Private append storage now creates exclusively and, on `AlreadyExists`, opens
the existing inode without creation flags. Both paths retain descriptor-relative
link, ownership and permission checks; an existing file is never truncated or
replaced. This also protects other private append users such as plugin locks
and diagnostic logs. The existing concurrent identity test covers the context
storage caller. `concurrent_append_creation_keeps_one_inode_and_every_write`
and `append_existing_rejects_links_and_preserves_unadmitted_files` in
`src/private_storage/tests.rs` cover simultaneous creation, retained append
contents and inode identity, private permissions, and unchanged linked targets.

Validation: 3,740 Rust tests pass with 34 existing ignored tests; formatting and
warnings-as-errors Clippy pass. Canonical workspace line coverage is 91.83%,
above the unchanged 89% floor. All 29 plugin conformance suites pass, with eight
existing real-mpv cases skipped locally because mpv is unavailable. All 18 bridge
tests pass, including two real MCP clients and two actual editor hosts. The
[performance register](../../reference/startup-performance.md) retains nine
complete workload samples and 360 steady-state input latencies.

Regression coverage includes:

- `profile_has_no_submission_approval_or_control_escape_hatch` in
  `src/workspace/context/tests/contract.rs`, plus terminal decoding/capture
  cases in `src/terminal/tests/read.rs` and delivery cases in
  `src/terminal/tests/proposal_delivery.rs`.
- `read_only_open_neither_creates_nor_repairs_storage` in
  `src/workspace/context/tests/storage.rs` and discovery tests in
  `src/workspace/context/discovery.rs`.
- `unsaved_unicode_reads_are_revision_checked_and_multiline_edits_undo_atomically`,
  `editor_read_alone_cannot_read_live_or_frozen_terminal_viewports`,
  `pane_a_to_b_to_a_never_revalidates_the_original_handle`, and
  `context_edit_does_not_save_or_complete_external_editor_wait` in
  `src/workspace/host/tests/context_reads.rs`.
- `real_pty_overlay_enter_only_inserts_and_separate_native_enter_submits` and
  `prepared_but_unpainted_pages_cannot_be_skipped_or_approved` in
  `src/workspace/host/tests/context_access.rs`;
  `native_command_prompt_requests_named_and_default_context_identity` in
  `src/app/tests/context_access.rs`.
- `two_live_hosts_are_discovered_and_reader_handles_do_not_cross_workspaces`,
  `regrant_changes_incarnation_and_old_events_cannot_disconnect_new_reader`, and
  `post_authentication_denial_is_delivered_before_connection_closes` in
  `src/workspace/host/tests/context_transport.rs`.
- `test_two_agent_clients_read_live_and_detached_workspaces_edit_unsaved_and_observe_revocation`
  in `bridges/runyte-context/tests/test_runyte.py`, with bounded fake-host/MCP
  cases in `test_bridge.py`, SDK/schema checks in `docs/plugins/check_context.py`
  and benchmark evidence checks in `benchmarks/test_context_access.py`.

Known limitation: this is a same-user permission boundary, not an operating
system sandbox against other processes running as that user. Printable input
can trigger actions in arbitrary terminal programs even without Enter. Native
macOS execution and coverage remain CI gates; local evidence here is Linux.
Package publication and agent-account configuration remain separate actions.

## Report

The following report preserves the pre-implementation behavior, requirements
and then-open design questions.

A coding agent running as a separate process cannot observe the editor state a
person is looking at. Guidance about the current file, the focused selection, or
the output of a command that just ran requires transcribing that state into the
agent's prompt by hand, and re-transcribing it after every change.

The state the agent needs is already editor state: open buffers and their text,
which pane shows what, where the selection is, and what a terminal session has
printed. Only buffer text has a durable on-disk equivalent, and even that is
stale for unsaved buffers. Terminal output, selections, and pane arrangement
exist nowhere else.

### Motivating arrangement

A person edits in a workspace while an agent runs in a terminal outside the
editor, or in a terminal belonging to a second workspace. The agent is asked
about work in progress. Answering well requires the text of the buffer being
edited, the selected range, and the tail of the terminal session where a build
or query last ran. None of it is reachable from the agent's process.

### Current behavior

The `runyte-1` application protocol already covers most of the read side. Under
`workspace`, `buffer.list` and `pane.list` issue editor handles without
scanning the filesystem. Under `text`, `buffer.read` returns exact
revision-bound text for a scalar range, and `buffer.snapshot.open/read/close`
serves explicit chunks of an immutable rope snapshot. Under `selections`,
`selection.get` returns an explicit pane, its displayed buffer, and a selection
revision.

`event.subscribe` accepts `{"kind":"buffers"}` for existing and newly opened
buffers, and explicit `{"kind":"buffer"}`, `{"kind":"pane"}` and
`{"kind":"attachment"}` handles, all requiring `workspace`. Baselines carry text
revision, accepted saved revision, dirty and read-only flags, scalar length, and
a bounded display label; pane metadata identifies its displayed buffer and its
selection revision. No text is copied into an observation. Ordering, coalescing,
resynchronization, and per-application source limits are specified in
`docs/plugins/applications.md`, under "Source subscriptions".

Three things are missing.

**Terminal content cannot be read.** `src/plugin/application.rs` defines sixteen
capabilities; none grants terminal reads. The `terminals` capability covers
`terminal.open` only, which returns an opaque handoff receipt and, as
documented, "does not grant plugin terminal-input or terminal-close operations".
Pane metadata reports a null displayed buffer for a terminal, so a terminal pane
is visible in an observation but its contents are not reachable.
`src/terminal/` holds a bounded scrollback and presentation cells, so the state
exists; there is no operation that exposes it.

**A native pane's visible region cannot be read.** Applications receive buffer
metadata plus explicit text reads at scalar offsets. There is no way to ask what
a pane is currently showing. A `{"kind":"viewport"}` source exists, but it
observes an application's own views rather than native panes. `src/snapshot.rs`
already produces owned, presentation-neutral snapshots for frontends; nothing
equivalent is offered over the protocol.

**No client exists, and cross-workspace addressing is undefined.** Applications
are configured per workspace and attach to that workspace's host. An agent
process outside the editor has no defined way to reach a workspace, and a
workspace ID is the unsalted SHA-256 of its project root, so it is not a usable
address for a caller that does not already know the root.

### Desired behavior

The approved implementation plan is recorded in
[Agent workspace context](../../plans/completed/PLAN_AGENT_WORKSPACE_CONTEXT.md).
It includes agents reading each other's terminal output in adjacent panes and
across multiple authorized live workspaces. At report time, the issue remained open.

Agents may edit buffers, including inserting newlines, with an explicit edit
grant. For terminal input, agents propose text for individual native approval
in an overlay showing the destination and exact text. The current proposal
recommends approval to insert text without Enter, preserving the requirement
that agents cannot submit commands. Approval to execute has not been authorized.

An agent process can obtain, on request, the set of panes and what each shows,
the text of a named buffer at a stated revision, the focused pane's selection,
and a bounded tail of a named terminal session, without the person copying any
of it.

The natural implementation is a bridge process that is an ordinary `runyte-1`
application on one side and speaks whatever protocol the agent already supports
on the other. No editor-core coupling to any particular agent is required, and
the bridge stays outside the editor's release surface.

### Constraints

Reads are pulled by the agent, not pushed to it. Subscriptions exist to
invalidate a bridge's cache, not to stream buffer contents outward. Continuous
push scales with the number of open buffers rather than with what was asked.

Exposing workspace contents to a process outside the editor is a new trust
boundary. The existing posture is per-workspace LSP permission records
(`src/lsp_trust.rs`, `:lsp-trust`), explicitly granted application capabilities,
and native-only approval of captured provider overwrites. An external reader of
every open buffer, including files holding credentials, belongs under the same
treatment: a distinct capability, granted per workspace, with a native
affirmative record rather than a configuration flag alone.

Every new read is bounded on the same terms as the existing ones: explicit
ranges, revision-bound results, and documented byte and line limits. A terminal
read states its bound in lines or cells and reports truncation rather than
returning an unbounded scrollback.

Nothing in this work writes runtime state under `context/`, and the bridge's own
state, if any, belongs beside other runtime state.

Terminal proposals send no input before approval. Approved insertion must not
include Enter, pasted line breaks, or control-sequence alternatives that submit
input. Buffer newlines remain allowed. Generic terminal programs may act on
ordinary characters; a no-Enter contract must not claim to prevent all child
actions. Buffer edits do not implicitly save or complete external-editor waits.

### Open questions

Whether terminal reads are a new capability or an extension of `terminals`, and
whether a read is addressed by pane handle or by terminal session handle.

Whether the visible-region read is a separate operation or a parameter on an
existing buffer read, given that `src/content_alignment.rs` keeps presentation
offsets out of the buffer and a row means the same thing at every pane size.

Cross-workspace access is required as well as reads within one workspace.
The proposal recommends an external bridge connecting to separately authorized
scoped endpoints, with explicit live-host identity and discovery. Its
admission and authorization contract must be finalized before implementation.

Whether an agent-visible read should be observable by the person, for example as
a notification or an indicator, so that external reads are not silent.
