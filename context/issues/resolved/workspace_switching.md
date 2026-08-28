---
title: "Workspace switching lacked a complete lifecycle and stacked client processes"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: 43a2058
---

## Resolution

Commit `43a2058` (`Complete switchable workspace lifecycle`) completed the
switchable-workspace surface. The existing worktree switch was launching and
waiting for a replacement client, so every round trip between workspaces grew
a process stack. The attached client now owns the transition loop and keeps its
terminal alive while it resolves or starts the destination host and reconnects.

The remaining lifecycle operations were private blocking helpers in
`src/main.rs`, which made them unusable from editor commands and unsafe on the
render path. They now live behind shared lifecycle functions and a bounded
Tokio workspace service. Registry scans run off the editor task, isolate
malformed rows, guard stale-row deletion by PID, and bound each host inspection.
The service merges live registry state with a bounded per-user recents cache,
and generation-tagged events rebuild the shared picker without losing its
filter or selection.

The four colon commands share that service and the existing result-list and
context-action overlays. Direct attachment and the worktree picker use one
persistent-only switch request: a persistent host may retain dirty work while
its client leaves, but stop and restart refuse to discard unsaved buffers.
Standalone Runyte can still inspect and manage Git worktrees without replacing
its in-process host. `workspace.mode: persistent`, safe idle retirement,
running/stopped CLI inventory, and the `--*-workspace` spellings complete the
lifecycle.
Platform-specific commands deliberately remain discoverable on non-Unix
systems and report that persistent hosting is unavailable, following the
project-wide platform policy rather than the report's original compile-out
proposal.

Coverage is provided by
`workspace_picker_keeps_filter_and_routes_enter_and_tab_by_row_identity` in
`src/app.rs`, `recents_are_deduplicated_most_recent_first_in_injected_storage`
and `empty_injected_registry_refreshes_without_touching_user_state` in
`src/workspace/catalog.rs`,
`idle_retirement_requires_clean_buffers_and_no_pending_wait` in
`src/workspace/host.rs`, `malformed_registry_rows_do_not_hide_valid_hosts` in
`src/workspace/transport.rs`,
`workspace_state_defaults_and_accepts_the_original_root_spelling` in
`src/config.rs`, `parses_host_listing_naming_restart_and_selected_shutdown` in
`src/launch.rs`, and
`persistent_worktree_switch_detaches_to_a_new_root_without_retargeting_the_host`
plus `switching_back_and_forth_keeps_one_client_process` in
`tests/local_protocol.rs`.

A 2026-08-26 test-portability follow-up replaced delays that guessed when the
asynchronous Git projection had settled with a control-protocol barrier on the
populated `[git worktrees]` buffer. A 2026-08-28 follow-up removed the remaining
startup delay as well: the real-TUI tests now wait until the rendered status
line contains the repository's current branch before typing `:git-worktrees`.
That branch is unavailable until discovery and the initial refresh complete,
so the command cannot race its own availability on a slower macOS runner.
Navigation still waits until the generated view contains the destination path.

A 2026-08-28 lifecycle follow-up separated ordinary quit from explicit detach.
`:q` from the last pane, plus `:qa` and `:qh` from any layout, now stop a
persistent session after the same unsaved-buffer, pending-wait, and
live-terminal safety checks used by host shutdown; `:detach` remains the
operation that preserves host state. The same follow-up stopped catalog refresh
results from creating a session-manager overlay unless that manager was already
open, so removing another worktree and its persistent session no longer
interrupts the current workspace.

A later 2026-08-28 CI follow-up removed the local-protocol test's remaining
passive waits for unsolicited Git frames. The test now polls current complete
frames through `Resynchronize`, uses the populated `[git worktrees]` buffer as
the service-completion barrier, and confirms the selection input through a
subsequent protocol round trip. Git discovery can still advance the optimistic
frame revision between receiving a snapshot and invoking the command, so a
typed stale-frame refusal now resynchronizes and retries within that state
deadline. Correlated host replies retain their five-second deadline, while
asynchronous Git state has its own thirty-second deadline and names the phase
that failed. The shared response helper also retains its caller in timeout
diagnostics, and the macOS job enables backtraces for child-process lifecycle
failures.

Known limitation: a switch does not forward file targets from the original
command line, and a running destination host keeps the configuration it loaded
at startup until it is restarted or retires.

## Report

### Workspaces as switchable attachment targets

The report began as a discussion note and became an open issue after the design
was agreed and implementation authorized. It replaced `discussion/automatic_server_
attachment.md`, which proposed a per-user broker or daemon routing clients to
per-root hosts. The direction below reaches the same goal — the convenience of
an Emacs-daemon workflow — without a broker, a supervisor process, or a shared
crash domain. Each workspace keeps its own host exactly as V8 defines it, and
the *client* does the routing.

## Motivation

A person working across several directories at once — a project, a linked
worktree, a separate notes or journal directory — currently keeps one Runyte
per tmux pane or tab. With per-directory hosts already implemented, each of
those directories can instead keep its state alive in its own host, and a
single client can move between them. Switching workspaces then replaces
switching terminals.

## The concept

A **workspace** is a directory Runyte recognizes as a project: marked by
`.git` or by the configured relative state directory (`.runyte/` by default),
and found by `project_root::discover`. Running or stopped is a **state** of a
workspace, not what makes one.

A **host** is the process that runs a workspace. Standalone Runyte runs one
in-process without publishing an endpoint. V8's boundaries are unchanged: one
host per workspace, one interactive TUI per host, and no host is ever
retargeted between unrelated roots.

The alternative reading — that a workspace *is* the live host, so a stopped
directory is not a workspace — was considered and rejected. Under it,
`:workspace-stop` would read as destroying the workspace rather than stopping
its host, and a workspace list could never contain a project that has not been
started yet, which is exactly when jumping to one is most useful.

## Vocabulary

`workspace` already carries this meaning in most of the tree (`WorkspaceHost`,
`WorkspaceIdentity`, workspace search, and the worktree list's "Open this root
as a separate Runyte workspace"). It also has an LSP-spec sense inside
`src/lsp/` — `WorkspaceEdit`, `WorkspaceSymbols`, `workspaceFolders` — which is
not ours to rename and stays as it is.

The one genuine collision is the word `root` in the state-store sense.
`resolve_workspace_root` returns `<project>/.runyte`, so `workspace_root` and
"the workspace's root" name two different paths, one nested inside the other.
A reader of `workspace.root: .runyte` under this vocabulary would conclude
their workspace root is `.runyte`, when it is the project directory.

Proposed renames, which are the whole of the vocabulary work:

- `workspace.root` → `workspace.state`, keeping the `workspace:` config
  section and accepting the old key as a serde alias. It then reads correctly:
  the workspace's state lives here.
- `workspace_root` → `state_root`, and likewise `resolve_workspace_root`,
  `validate_workspace_root`, and `configured_workspace_root`. Roughly 45 sites,
  concentrated in `src/main.rs`, `src/project_root.rs`, and
  `src/file_picker.rs`.
- `workspace_root_bytes` → `project_root_bytes` in `src/protocol/mod.rs:199`.
  The field is filled from `endpoint.project_root`, so it is a misnomer today
  regardless of this proposal.

`project_root` is unchanged; it is already the unambiguous name for the
workspace directory, so no "workspace = project directory" rename is needed in
code. `WorkspaceHost`, `WorkspaceIdentity`, and `SwitchWorkspace` are correct
under the unified meaning and stay. Keybindings are unchanged, including
`Space s w`.

## The architectural prerequisite

Switching workspaces is already implemented, reachable only from the worktree
list: `Space g w` and Enter sets `App::workspace_switch`, the host answers
`HostResponse::SwitchWorkspace` and drops the client, and the client reopens
against the destination root.

It reopens by re-exec. `launch_workspace_process` spawns a child
`runyte --attach` and blocks on `command.status()`, so switching A to B leaves
A's process alive waiting on B, and switching back stacks a third. For a
switcher used many times a day this is an unbounded process chain in which
quitting unwinds a stack, and every switch tears down and re-enters the
terminal.

The client should instead loop: `run_attached` returns a destination, the
client re-resolves the endpoint and reattaches in-process, holding the
`TerminalGuard` across the transition. This removes the nesting and the
flicker, and improves the existing `Space g w` path. Two consequences follow:

- `run_attached` currently fails the process on a `Refused` response. Once
  switching is a keystroke, "another interactive TUI is already attached"
  becomes routine, and must leave the client where it is with a status message
  instead.
- Client-side configuration — effectively only `editor.mouse`, which the
  terminal setup needs — comes from the launching process rather than from the
  destination workspace, since the host owns everything else.

## Command surface

- `:workspace-list`, alias `:wls`. A popup listing known workspaces, the
  current one marked `*` as the worktree list already marks its current root.
  Each row shows name, directory, running or stopped, dirty-buffer count, and
  whether a TUI is attached elsewhere. Enter attaches, starting a host if the
  workspace is stopped. Tab opens contextual actions, initially only Close.
  This is the buffer picker's existing Enter-opens/Tab-acts contract.
- `:workspace-attach PATH|ID|NAME`, alias `:wat`. Attaches, starting a host if
  none is running there. Path completion is structural: declaring
  `CommandArguments::Required(ArgumentKind::Path)` provides it.
- `:workspace-start PATH`. Starts a host and stays in the current workspace,
  for warming up a project in the background. Named `start` rather than `new`
  because `new` implies switching to it.
- `:workspace-stop [PATH|ID|NAME]`. The same path as the popup's Close.

Command-line flags gain matching `--list-workspaces` and similar spellings,
with the existing `--list-hosts` family retained as aliases, so the `:command`
and the flag never use different nouns for the same thing.

## Configuration

`workspace.mode`, one of `standalone` (default) or `persistent`. In persistent
mode a bare `runyte` starts a host in the discovered project and attaches to
it; `:quit` from the last pane detaches and ends the client, and only a
workspace switch reattaches. `--standalone` overrides the configured mode, and
`--attach` and `--serve` remain explicit spellings of what the mode otherwise
picks.

Persistent mode stays opt-in deliberately. A host that outlives the terminal
because someone typed `runyte` is precisely the surprise that
`automatic_server_attachment.md` set out to avoid.

## Safety

Stopping a host discards unsaved edits until disk-backed recovery exists, and
this proposal puts that one Tab-and-Enter away from every project a person has
open. Close and `:workspace-stop` must therefore refuse, or require a separate
confirmation, when the target has dirty buffers — the protection the buffer
picker's Discard action already applies. This is why the dirty-buffer count
belongs in the row rather than in a later prompt.

Switching *away* from dirty buffers remains allowed, because the old host
keeps them: `App::enable_persistent_workspace_switch` already draws that
distinction.

## Platform

`registered_hosts` and the rest of `src/workspace/transport.rs` are Unix-only,
and persistent hosting is documented as Unix-only. Because `src/keymap.rs` is
the single source for dispatch, help, and key hints, these commands must be
absent from the registry on other platforms rather than present and failing,
so help never advertises a command that cannot run. `workspace.mode:
persistent` should be rejected there with a clear message.

## Already implemented

- `runyte --shutdown-host` with no argument, which resolves the host from the
  project discovered at the current directory. The proposal's request for this
  is already satisfied, with the refinement that it selects the discovered
  project root rather than the literal current directory.
- The host registry and its selectors: full ID, unambiguous ID prefix, exact
  name, or project directory.
- Starting a detached host, as `--restart-host` already does.
- Structural path completion for command arguments.
- The switch mechanism itself, subject to the re-exec problem above.

## Decided since this was written

- **Stopped workspaces come from a recents file**, not a filesystem scan. A
  scan would need a new config key for where to look, cost I/O on every open,
  and surface hundreds of irrelevant repositories. The trade-off accepted is
  that a workspace never opened before does not appear until
  `:workspace-attach` reaches it once.
- **Hosts created by `--wait` are listed** like any other. A host started by
  `git config core.editor 'runyte --wait'` holds a real workspace and may well
  need stopping, and hiding it would require a new metadata field and leave a
  host that exists yet is invisible.
- **The command-line flags gain `--*-workspace(s)` spellings** in the same
  release, with the existing `--*-host` family retained as aliases, so the
  `:commands` and the flags never use different nouns for one thing.
- **Configuration after a switch**: the destination host owns its own editor
  configuration. The client contributes only what the terminal needs, in
  practice `editor.mouse`.
- **`:quit-here` works in client-server mode and across a switch**, and belongs
  to this issue rather than a later one. `--cwd-file` is currently refused
  outside standalone mode, so a shell wrapper that always passes it would stop
  `runyte` from starting at all once persistent mode is enabled: this is a
  prerequisite of the mode, not an enhancement. The split is already clean —
  `--cwd-file` is a client argument and the client writes the file, while the
  editor running in the host owns the chosen directory. The host therefore
  reports that directory when it honours the detach, and the client writes it
  exactly as standalone does. Whichever host the client is attached to reports
  its own directory, which is what makes this work across a switch.
- **In persistent mode `:quit-here` detaches and leaves the host running**,
  exactly as `:quit` does from the last pane. Letting it stop a clean host
  instead would make the navigation workflow self-cleaning, but it would give
  `:qh` and `:q` different meanings in the same mode, and coherence between
  them matters more. Idle retirement reclaims those hosts instead.
- **Idle hosts retire on a timer.** A host exits when no client has been
  attached for the configured interval, it has no unsaved buffers, and no
  `--wait` request is outstanding. Those conditions make it safe: a workspace
  with unsaved work pins its own host, so nothing is ever lost. The interval is
  configurable and defaults to 1440 minutes, one day — a person may leave a
  machine running for a long time across many worktrees, and a short default
  would retire hosts they are still using. This is also what keeps the
  `:quit-here` navigation workflow from accumulating hosts: a directory visited
  once and left clean reclaims itself.
- **Platform-specific commands stay in the inventory and report themselves
  unavailable**, rather than being compiled out off Unix. See
  `windows_support.md` for the reasoning; it applies to any later
  single-platform feature too.

## Open decisions

- **Whether a switch forwards the requested file targets.** It does not today,
  which is only visible in that files named on the command line are not carried
  into a destination workspace.
- **How a configuration change reaches an already-running host.** The file is
  read once at startup and there is no reload path, so a host keeps the
  configuration it started with. Idle retirement means a host eventually picks
  up a changed configuration on its own, which reduces but does not remove the
  need for an explicit reload.
