# Runyte V8 asynchronous workspace and Git

Status: completed 2026-08-12

Created: 2026-08-11

## Decision

V8 introduced one editor host that can run in-process for standalone mode or
inside an optional persistent local process. Slow workspace services report
bounded events back to that host rather than blocking input or rendering. The
same cycle completed Runyte's built-in Git views and revision-safe file handoff
through `--wait`.

The design preserves one editor implementation in both deployment modes:

```text
standalone TUI ──> WorkspaceHost in the same process

attached TUI ───> private local protocol ───> persistent WorkspaceHost
```

The protocol is private to bundled local clients. It is versioned and bounded,
but it is not a public RPC, remote-execution, or extension contract.

## Workspace ownership

A workspace is one project-root editor scope. `WorkspaceHost` owns the live
editor, buffers, panes, selections, registers, language services, Git state,
and terminal sessions. Standalone mode keeps that host in the TUI process.
Persistent mode keeps it in a local host so an attached TUI may disconnect
without discarding the workspace.

Persistent-session discovery uses canonical workspace identity and local
registration data. Transport endpoints, locks, and retained runtime state live
under the ignored `.runyte/` boundary or the configured platform runtime
location, never under tracked `context/`.

Only one interactive TUI may attach to a persistent session at a time. The
host is local and does not claim survival across process termination, logout,
reboot, or machine failure.

## Asynchronous service boundary

Potentially slow operations run behind explicit services and bounded queues.
The editor initiates work, remains responsive, drains results between frames,
and applies a result only when its identity and revision still match live
state. Coalescing is used where only the newest observation matters; ordered
mutation workflows retain FIFO meaning.

Immutable editor snapshots cross the frontend boundary. Frontends render
semantic values without reading live editor state, while commands return to
the host for execution through the same registry used in standalone mode.

## Git boundary

`src/git/` is the only module that runs Git. It uses argument vectors rather
than a shell, bounds subprocess output, and converts results into structured
values. The service covers status, branches, worktrees, history, blame,
stashes, fetch/pull/push, and revision content.

Open-file gutter marks are computed from cached staged text in memory. Status
buffers and pickers consume the same structured service rather than spawning
their own processes. The line-diff alignment in `src/diff.rs` is shared by the
gutter and side-by-side comparison so the two views cannot disagree about
which lines correspond.

Partial staging is revision-safe: a reviewed selection is tied to the file and
index state from which it was prepared, and stale application is refused.
Worktree operations keep path arguments structured and preserve the editor's
workspace boundary.

## Wait requests

`--wait` is the target-bearing handoff into a persistent session. Each request
tracks the buffers and revisions it opened and completes only through an
explicit, revision-safe close or completion action. A client disconnect or a
newer modification cannot be mistaken for successful completion.

## Current records

Current user behavior is documented in `README.md` and
`docs/user-guide.md`. `context/reference/ui-vocabulary.md` defines the
presentation terms inherited by attached frontends, and the architecture map
in `AGENTS.md` records current module ownership.

This plan is an implemented architecture record. Later changes may refine the
details; the current source and reference documents remain authoritative.
