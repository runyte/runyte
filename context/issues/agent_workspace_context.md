# Expose live workspace context to external agent processes

A coding agent running as a separate process cannot observe the editor state a
person is looking at. Guidance about the current file, the focused selection, or
the output of a command that just ran requires transcribing that state into the
agent's prompt by hand, and re-transcribing it after every change.

The state the agent needs is already editor state: open buffers and their text,
which pane shows what, where the selection is, and what a terminal session has
printed. Only buffer text has a durable on-disk equivalent, and even that is
stale for unsaved buffers. Terminal output, selections, and pane arrangement
exist nowhere else.

## Motivating arrangement

A person edits in a workspace while an agent runs in a terminal outside the
editor, or in a terminal belonging to a second workspace. The agent is asked
about work in progress. Answering well requires the text of the buffer being
edited, the selected range, and the tail of the terminal session where a build
or query last ran. None of it is reachable from the agent's process.

## Current behavior

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

## Desired behavior

The approved implementation plan is recorded in
[Agent workspace context](../plans/active/PLAN_AGENT_WORKSPACE_CONTEXT.md).
It includes agents reading each other's terminal output in adjacent panes and
across multiple authorized live workspaces. The issue remains open.

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

## Constraints

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

## Open questions

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
