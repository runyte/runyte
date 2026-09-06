# Session and open-destination navigation

Status: proposed; awaiting final UX review before implementation.

Created: 2026-09-06

## Objective

Make Runyte sufficient for a workflow that currently uses tmux windows to hold
different worktrees, with Shift-Left/Right switching between those windows.
Running persistent sessions should be visible without opening a picker, open
buffers and terminals should share one navigation surface, and opening another
directory as a persistent session should not require typing a lifecycle command.

Keep the editor central: add one quiet global row and use temporary overlays
for detailed navigation. Avoid a permanent sidebar or a second row of tabs for
every open buffer and terminal.

This document proposes behavior. It does not change the current user guide or
authorize implementation before final review.

## Navigation model

There are two levels of destinations:

- A persistent session is a complete work context associated with one workspace
  root, often a Git worktree. Switching it restores that context's panes,
  buffers, and terminal sessions through the existing attachment mechanism.
- Inside a workspace, open buffers and terminal sessions form one navigable
  working set. Panes are the layout in which these destinations appear.

Combining destinations in navigation does not make a terminal a buffer or
change their different close, persistence, and process-lifetime semantics.

| Surface | Question it answers | Entry point |
| --- | --- | --- |
| Session strip | Which persistent sessions are running, and which is current? | Visible global row |
| Session manager | Which persistent session should be opened or managed? | Existing `Space Space` |
| Navigator | Which already-open buffer or terminal should be visited? | New `Space n`, alias `Ctrl-w n` |
| Finder | Where is a file, buffer, terminal, or matching content? | Existing `Space f` |

The Navigator works in standalone and persistent modes. Persistent-session
controls retain their existing persistent-mode capability requirement; this
plan does not introduce automatic conversion of a standalone editor.

## Session strip

Place a single row above the editor area. Keep the existing global status and
interaction lines in their current roles.

```text
  1 main    [2 parser-fix]    3 docs ·    4 experiment
```

The brackets illustrate current-session emphasis, not required literal styling.
Each entry contains the existing session number, when assigned, and its stable
session name. Running unnumbered sessions remain reachable. Stopped history
stays in the session manager.

Use the manager's ordering and numbering rules. Do not reorder entries because
of output or visits. Explicit renumbering and host starts/stops may change the
order according to the existing catalog contract. Show an overflow count such
as `+3` when necessary, keeping the current entry visible. The overflow count
must not suggest that only numbered sessions exist.

Proposed visibility setting:

- `auto` (default): show when more than one persistent session is running.
- `always`: show even with one persistent session.
- `hidden`: omit the strip while retaining navigation commands.

Explicit zen presentation hides the strip. Resolve that behavior alongside the
existing zen/fullscreen geometry; do not repeatedly resize panes for activity
changes. Only a change in whether the row is present changes available height.

### Activity without a dashboard

Keep paths, branches, ages, counts, and process details in the manager. The
strip has no animation or changing terminal titles.

Distinguish three facts: the current attachment, a running host, and unseen
terminal output. A subdued marker may summarize unseen output; a bell or a
confirmed host connection failure may receive stronger emphasis. These must
have explicit meanings and accessible text in the manager, not rely on colour
alone. A refresh timeout means unknown health, not proof that a host stopped.

Output does not establish that a command or agent is working, waiting, or
finished. Preserve the current meaning of `QUIET` in detailed session status:
an observation about completed terminal output lines, not inferred job state.
Do not relabel it as idle or complete.

Before implementing attention markers, specify their acknowledgment behavior.
Recommended default: aggregate terminal unread/bell state and clear it through
the terminal's existing viewing/acknowledgment rules. Visiting a persistent
session must not silently acknowledge output in every hidden terminal.

## Direct persistent-session navigation

| Action | Proposed binding or entry point |
| --- | --- |
| Previous running persistent session | `Shift-Left` |
| Next running persistent session | `Shift-Right` |
| Jump to a numbered persistent session | Existing `Space Space`, then `1`–`9` with an empty filter |
| Inspect or manage persistent sessions | Existing `Space Space` |
| Return to the previously visited persistent session | New semantic command; binding to settle at review |

Previous/next follows the strip order, includes unnumbered running sessions,
wraps at the ends, and never starts stopped history entries. One eligible
session produces no switch. An unavailable destination must not be forcibly
attached or restarted. Retain existing attachment protections, including the
single interactive client contract.

The previous-session command alternates between the last two successful
attachments. Failed switches do not overwrite that history. It is separate
from walking the strip's order.

Shift-Left/Right should work from editor Insert and Terminal Insert as well as
Normal/Select, without requiring a mode change first. These become explicit,
remappable exceptions to terminal key forwarding in persistent mode. Existing
overlays and confirmations keep their input-ownership contract; navigation
must not abandon an unresolved operation by bypassing it.

During migration from tmux, an outer tmux that binds these keys consumes them
before Runyte can receive them. Document releasing or changing those outer
bindings. Do not modify personal tmux configuration as part of implementation.

## Navigator

### Entry points and presentation

`Space n` opens the Navigator immediately in Normal/Select. It is a complete
binding, not a namespace requiring a third key. The hint description is
`Navigate open buffers and terminals`.

`Ctrl-w n` invokes the same command through the existing pane prefix, including
from Terminal Insert. Honour `keys.window` remapping. `Space n` continues to
be ordinary child input in Terminal Insert. Make the pane-prefix alias
available from editor modes too so it is a consistent navigation gesture.

The surface is a picker overlay titled `Navigator`, described as `Open buffers
and terminals`, with the current workspace or persistent-session name as
context where space permits.

```text
Navigator — parser-fix
> type to filter

▸ [file]     src/parser.rs       modified · pane 1
  [terminal] tests              running  · pane 2
  [terminal] shell              running
  [file]     README.md
  [explorer] .
  [git]      status
```

The mockup illustrates information density. Actual labels use the existing
structural buffer and terminal vocabulary and existing modification markers.
Pane references describe current visibility; they do not introduce permanent
pane numbering as a new feature.

Include ordinary and pathless buffers, retained special buffers, and running
terminal sessions. Exited terminal sessions are excluded from the Navigator
and remain available for review and management through `Space t t`. Deduplicate by
resource identity, recording all visible panes for shared buffers. Respect
existing special-buffer retirement; the Navigator must not retain extra views
merely because they appeared in its list.

Do not scan the filesystem or include unopened files. Do not search document
text or terminal output. Keep specialized buffer and terminal managers available
for their existing workflows.

### Browsing and fuzzy matching

An empty query shows the open working set immediately. Proposed default: order
by recent activation on opening, select the current destination initially, and
keep that order stable for the lifetime of the open overlay. Clearing the query
restores that order. Terminal output alone never changes it.

Typing fuzzy-matches buffer names and paths, and terminal names, titles, and
launch commands. Prefer exact names, prefixes, and contiguous substrings over
scattered subsequences; highlight the characters that matched. Preserve stable
ordering for equally ranked results. There is no substring/fuzzy toggle.

Reuse the existing fuzzy matching primitives and Finder name-field semantics
where applicable. Path fields should retain useful basename and path-component
weighting; terminal labels should not be interpreted as filesystem paths. The
Navigator's narrower candidate set must not acquire a second query language.
For clarity, `Space g f` already uses fuzzy matching over commit rows; the
distinction from the Finder is scoring and scope, not substring versus fuzzy.

| Query | Example destination |
| --- | --- |
| `parser` | `src/parser.rs` |
| `sprs` | `src/parser.rs` |
| `tests` | Terminal named `tests` |
| `readme` | `README.md` |

Up/Down, `Ctrl-p`/`Ctrl-n`, and existing paging keys move the selection. Enter
visits it. Tab opens contextual actions for its resource type, reusing existing
Save, Close, Rename, and related semantics where applicable. Printable `j`,
`k`, and `q` filter rather than becoming navigation or dismissal commands.
Escape, `Ctrl-c`, or Space with an empty query dismisses the overlay. A space
after query text participates in fuzzy matching under the existing conventions.

Canceling a Navigator opened from Terminal Insert restores that input context
without forwarding overlay input to the child or capturing terminal review.
Accepting uses the established destination focus/mode rules: a document starts
in Normal, a live terminal normally receives Insert, and a captured terminal
review remains in review.

### Visiting and returning

Proposed default, requiring final review:

- If the destination is already visible, focus its pane. If several panes show
  the same buffer, prefer the active pane, then a deterministic recent pane.
- Otherwise show it in the active pane, retaining the pane layout.
- Provide an explicit contextual action to bring it into the active pane when
  that differs from ordinary focus.

This deliberately differs from the current terminal manager's Show action,
which moves a visible terminal into the active pane. Preserve that manager's
existing behavior unless separately approved. Never create two live views of
one PTY.

Add a previous-destination semantic command spanning buffers and terminals,
with a binding to settle at review. Recommended scope: the active pane's
destination history, so edit → terminal → edit is repeatable without surprising
changes to other panes. Closed or retired destinations and exited terminal
sessions are skipped. Any later sequential cycling should use a stable order
separate from recent-use ranking.

### Cleaning up exited terminals

Keep exited terminal sessions in the existing `Space t t` terminal manager,
including their retained output and individual review/close actions. Add
`Close all exited terminals` to its Tab action menu. The action applies to the
current workspace and is available regardless of whether the selected row is a
running or exited terminal. Disable it with an explanation when there are no
exited terminals to remove.

Apply the existing removal semantics for an exited terminal to every eligible
terminal: remove its retained entry and output, and safely replace any pane
still reviewing it using the established close behavior. Running terminals
remain untouched; this action never terminates a live child. Determine eligible
terminal identities from authoritative state when executing the action, not
from filtered row indices. Report how many exited terminals were closed and
keep the manager open with a valid selection or its empty state.

## Opening a persistent session from a directory

The session manager exposes `Open directory…` even when no row is selected or
the list is empty. It must be a keyboard-reachable manager-level action, not
an action dependent on a selected session. Its exact shortcut remains to be
chosen without taking printable characters away from filtering.

```text
Space Space → Open directory… → choose directory → attach
```

Use a directory chooser with recent roots, sibling Git worktrees, typed path
completion, and directory browsing. Make choosing the current directory
explicit and distinguish it from descending into a child. Preserve the familiar
`~`, relative-path, and absolute-path spellings. Cancellation leaves the current
workspace and attachment unchanged.

Accepting an existing directory reuses its persistent session or initializes
that exact workspace root and starts its host through the current
`:session-attach` workflow. It does not silently select an ancestor workspace.
Normal initialization, LSP trust, and attachment checks continue to apply. A
failure leaves the existing attachment usable and explains the failed step.

### From the explorer

Include a Tab-menu action named `Open persistent session here` for the directory
the explorer is currently browsing. It is available even in an empty directory;
it does not depend on a selected child entry.

```text
Explorer → navigate into a directory → Tab → Open persistent session here
```

The action attaches the current TUI to that exact directory's persistent
session, starting it when necessary. The destination appears in the session
strip according to its normal visibility rules. Existing sessions are reused,
so the label says Open rather than promising a new duplicate session.

A selected directory entry may additionally expose `Open selected directory as
persistent session` without first descending into it. Keep that label distinct
from opening the explorer's current directory; never infer the target from an
ambiguous “here.”

### From an integrated shell command

Support invoking `runyte -a` from an integrated terminal as a request to switch
the outer Runyte TUI. `ru -a` has the same behavior when `ru` is a shell alias
or wrapper that invokes Runyte; this feature does not require that alias.

```sh
cd ../parser-fix
runyte -a
```

With no workspace argument, use the invoking CLI process's actual current
directory as the exact destination root. `runyte -a ../docs` resolves its
relative directory argument against that process's current directory; supported
session names and IDs retain the normal explicit-selector semantics.

The CLI detects its Runyte-owned integrated-terminal context and sends a bounded
local attachment request to the owning host. The outer TUI reuses or starts the
destination persistent session, adds it to the normal inventory/strip, and
immediately attaches there. The invoking CLI returns to its shell without
starting a nested TUI or taking over the PTY's display. The original shell,
terminal session, and persistent host remain alive; switching back returns to
the same shell in the directory it entered.

This command route reads its own working directory and therefore works without
OSC 7 shell integration. It is a required opening flow, distinct from the
editor-side terminal action below. Ordinary `runyte -a` outside an integrated
terminal retains its existing launch behavior.

Implement this as a dedicated private control request, not unrestricted
semantic-command access for CLI control clients. Supply and validate the
owning-host/terminal identity through the PTY launch context. An environment
marker alone is not authority to switch an arbitrary TUI. Route the request
through the existing attachment workflow and its initialization, trust, and
single-interactive-client checks; never rewrite the old workspace root.

Bind the handoff to the originating terminal and current attachment, so a stale
child environment or a delayed request from a different work context cannot
redirect whichever TUI happens to be attached later. Resolve reply lifetime,
completion/error reporting, and duplicate requests explicitly: the old host
must be able to answer its child even after the interactive TUI switches away.
Do not claim completed attachment merely because a request was queued.

If a recognized parent context is stale, incompatible, standalone, detached,
or unable to honor the request, return an actionable error and preserve the
current context. Do not silently fall back to a nested editor after a failed
handoff. Define any deliberate nested-launch override separately before adding
one. Ordinary file opening retains its existing semantics. Parent-routed
`--wait` editing is a separate request lifecycle described below; it does not
perform a persistent-session switch.

### Editor-side terminal action and worktree creation

Also offer `Open this terminal's directory as persistent session`, using the
active terminal's last validated reported directory. When that value is
unavailable, explain why; do not guess from prompt text. This is a convenient
editor action, while `cd` followed by `runyte -a` is the direct shell workflow.

Expose creation of a new Git worktree from the session manager by reusing the
existing `Space g w` create-and-attach operation and its protections.

Opening a persistent session for an existing directory does not create a new
directory or worktree. Worktree creation remains an explicit Git action.

## External-editor requests in the parent Runyte

When a program in an integrated terminal invokes `runyte --wait <file>`, route
the editing request to the terminal's owning persistent host through the same
validated parent context used for `runyte -a`. This extends the existing wait
mechanism and its covered-terminal return behavior. It must not depend on the
caller's current directory discovering the right workspace, or treat a prompt
file under a temporary directory as a new workspace.

Use an ordinary editable buffer in the current persistent session, temporarily
covering the originating terminal in its pane. Do not create a temporary
persistent session. The caller waits while the buffer is edited, then receives
completion and resumes in the same terminal. The buffer can participate in
normal navigation and splits while its request remains pending.

Typical flow:

```text
Agent terminal → external editor request → parent Runyte buffer → :wq
              → save and finish → original agent terminal
```

Document `EDITOR='runyte --wait'` and `VISUAL='runyte --wait'` as the external
editor configuration. Do not intercept the agent's Ctrl-g or require knowledge
of a particular agent's shortcuts. The calling program remains responsible for
launching the editor and reading the edited file after completion.

### Saving and returning to the terminal

For a prompt buffer owned by a pending parent-routed external-editor request,
these three routes have the same successful outcome:

| Route | Behavior |
| --- | --- |
| `:wq` | Save the prompt buffer, complete its edit, and return to the agent terminal |
| `:wbc` | Save the prompt buffer, complete its edit, and return to the agent terminal |
| `:w` followed by `:q` | Save first, then complete the edit and return to the agent terminal |

The third route means two separately executed commands, with no intervening
unsaved edits. `:w` alone saves and keeps the request pending. Bare `:q` or
`:quit` completes a clean request buffer without writing; if it has unsaved
changes, retain normal unsaved-change protection and keep the edit open. Quit
does not implicitly save.

All three successful completion routes preserve the parent pane, restore the
same originating terminal where the covered-terminal relationship is still
valid, and leave its agent process and persistent host alive. Do not write
unrelated buffers. For a multi-file request, the caller resumes only after
every requested file has been completed.

This is scoped completion behavior, not a global textual alias for `q`.
Record request ownership explicitly rather than inferring it from a temporary
path, file extension, or process name. Ordinary buffers and external `--wait`
invocations without the parent-routed editing context keep their current quit
semantics. Navigating to an unrelated buffer never transfers this behavior;
returning to a still-pending request buffer restores its editing context.

If a save fails or encounters an existing write
protection/conflict, leave the buffer and request open, show the error, and do
not report success to the waiting process. Identify this editing context in
help or a concise action hint such as `:wq saves and returns to terminal`.

Proposed cancellation counterpart: `:q!`/`:quit!` discards unsaved changes in
this editing context and cancels its request with a nonzero client exit status,
then returns to the terminal where possible. It does not undo an earlier
explicit `:w` or promise how the calling program presents cancellation. Retain
existing protection for buffers shared with other pending requests; one edit
must not discard another request's unsaved work.

Switching persistent sessions leaves the request pending in its owning host.
Define explicit detach and lost-caller cancellation behavior during protocol
design; a parent-routed wait must never fall back to drawing a nested TUI in
the originating PTY. Preserve lifecycle-loss detection and bounded waiting.

## Inspecting another persistent session's open destinations

Extend the session manager with an optional list of the selected persistent
session's open buffer and running terminal identities and states, using the
Navigator's eligibility rules. Fetch it on demand
for the selected host, rather than loading every host's inventory for the strip.
Keep stopped sessions, unsupported host versions, loading, and failed requests
explicit; none should look like a confirmed empty inventory.

Show names and states rather than miniature document or terminal contents. The
existing manager metadata remains available. The inventory must be a navigable
surface with explicit enter/back actions, not actionable rows hidden inside a
text-only preview. Exact layout and the key to enter it need final design.

Allow choosing a destination to attach to its persistent session and visit
that resource. Revalidate its identity after attachment. If it was closed in
the meantime, retain the destination session's restored layout and report that
the resource is no longer open; never resolve a stale row index to another
resource. This extends the existing bounded local protocol and can follow the
local Navigator and basic session navigation as a separate implementation step.

## Implementation boundaries and performance

Keep editor coordination in `src/app/`, session discovery and attachment in
`src/workspace/`, semantic snapshots in `src/snapshot.rs`, bounded local DTOs in
`src/protocol/`, and drawing in `src/ui.rs`. Reuse picker and fuzzy matcher
primitives without starting a Finder filesystem scan for the Navigator.

Define all actions, default bindings, aliases, capability checks, and terminal
prefix behavior through the shared command/keymap registry. Help, hints, and
remapped spellings must describe the commands actually executed.

The strip introduces observation while the session manager is closed. Design
its discovery and scalar refresh budget explicitly: asynchronous bounded
requests, coalescing, freshness limits, and no Git subprocess per rendered
entry or frame. Startup must not wait for all hosts. Unchanged observations
must not redraw the screen. Hidden/zen presentation should avoid attention-only
refresh work; automatic visibility still needs a bounded way to discover host
starts/stops. Host inventories remain on demand.

Use the normal catalog/environment scope, including its treatment of hidden
isolated hosts. Do not widen discovery to all namespaces as a side effect of
showing a bar. Rendering never reads other hosts' terminal contents.

Consult and update the current UI vocabulary, keymap reference, terminal
compatibility record, and startup-performance register as implementation changes
their contracts. Measure startup and idle cost with `benchmarks/`, including
one and several persistent sessions, a hidden strip, and a noisy terminal in
another persistent session.

## Delivery and validation

Suggested implementation sequence after approval:

1. Local Navigator, `Space n`/`Ctrl-w n`, fuzzy matching, destination focus,
   contextual actions, previous-destination navigation, and the terminal
   manager's `Close all exited terminals` action.
2. Session strip, direct previous/next switching, previous-session navigation,
   visibility settings, and bounded activity observation.
3. Directory chooser, explorer Tab action, integrated `runyte -a` handoff to the
   outer TUI, editor-side terminal action, worktree session-opening route, and
   parent-routed external-editor waits with equivalent save-and-return routes.
4. On-demand destination inventories across persistent sessions and direct
   attachment to a selected resource.

Each step updates the relevant user guide, references, help, and tutorial
hints. No issue is moved to resolved merely by approving this plan.

Behavior coverage must establish:

- Mixed open buffers and running terminals appear once by identity; unopened
  files and exited terminals do not appear. A terminal exiting while the
  Navigator is open disappears safely without retargeting a stale selection.
  Retired buffers and closed terminals likewise disappear safely.
- Exited terminals remain reviewable in `Space t t`. Its bulk cleanup removes
  all exited entries and retained output in the current workspace, including
  filtered-out entries, while preserving live children and handling panes that
  were reviewing removed terminals. Cover no eligible entries and child exit
  between opening the action menu and executing the action.
- Empty-query ordering, direct-match preference, fuzzy abbreviations, visible
  highlights, query cancellation, and selection identity survive updates.
- Navigator focus preserves layouts and PTY single-view ownership, and respects
  document, live-terminal, and captured-review mode transitions.
- `Ctrl-w n` works from Terminal Insert, cancellation resumes child input, and
  configured leader/window prefixes update dispatch, help, and hints together.
- Session cycling agrees with strip order through wraparound, unnumbered hosts,
  host changes, failed attachments, and stopped history entries.
- Strip overflow, narrow geometry, one-session visibility, zen presentation,
  unknown health, and attention acknowledgment remain understandable.
- All directory-opening routes reuse exact-root initialization and attachment
  behavior, survive cancellation/failure, and preserve the prior host and PTYs.
- The explorer's Tab action opens its currently browsed directory, including an
  empty directory, independently of the selected entry. A separately labelled
  selected-directory action uses that entry instead.
- `cd` followed by `runyte -a` in an integrated terminal switches the outer TUI
  to the CLI's exact current directory without starting a nested editor. Cover
  absent OSC 7 reporting, relative directory arguments, existing destinations,
  returning to the original live shell, stale parent context, changed
  attachment, unsupported protocol, failed destination attachment, and reliable
  child completion reporting across the switch. Outside an integrated terminal,
  the existing CLI launch behavior remains intact.
- Remote inventories are bounded, version-aware, and safe against stale
  resources, delayed replies, and host replacement during attachment.
- Parent-routed external-editor waits open in the originating persistent host
  and return to the originating terminal without creating a nested TUI or a
  temporary persistent session. Cover changed caller directories and temporary
  prompt paths. `:wq`, `:wbc`, and successful `:w` followed by `:q` have the same
  save-and-return outcome. Clean `:q` completes without writing; dirty `:q`
  protects unsaved changes, including edits made after `:w`. Failed saves leave
  the caller waiting; `:w` alone does not complete. Cover
  cancellation, multiple requested files, shared-buffer protections, navigation
  away and back, persistent-session switching, detach, caller loss, and unchanged
  ordinary quit behavior. Completion preserves the pane, host, and live child.
- Session observation does not block input, scan project files, or redraw
  unchanged frames; measured idle/startup costs are recorded.

Before handing off Rust implementation, run `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`. Preserve the
canonical `cargo llvm-cov --locked --workspace` floor in
`context/reference/test-coverage.md` on each affected first-class target.
Storage tests use temporary roots and checked-in executable fixtures.

## Final review decisions

The proposed direction is a session strip plus a transient local Navigator,
with `Space n` and `Ctrl-w n`, fuzzy identity matching, and visible ways to open
another directory as a persistent session. Confirm these remaining defaults
before implementing the affected behavior:

1. Focus an already-visible destination's pane, with bringing it into the active
   pane as an explicit action.
2. Use recent activation order on Navigator opening, frozen while open, and
   active-pane history for the previous-destination command.
3. Default strip visibility to `auto` and hide it in zen presentation; confirm
   attention markers and their acknowledgment rules.
4. Choose bindings for previous destination, previous persistent session, and
   the manager's `Open directory…` action without conflicting with input or
   existing registry bindings.
5. Settle the selected-session inventory's layout and enter/back interaction.
6. Confirm the external-editor cancellation counterpart (`:q!` cancels with a
   nonzero result) and behavior when the outer TUI explicitly detaches.
