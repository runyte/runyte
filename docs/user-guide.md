# Runyte user guide

This is the detailed reference for Runyte's editing model, language support,
persistent sessions, terminals, Git workflows, keybindings, and configuration.
For the project overview and quick start, see the [main README](../README.md).

## Feature reference

- Normal, Insert, Replace, Select, and Command modes
- Tree-sitter syntax highlighting for Python, Rust, Swift, C, C++, JavaScript,
  TypeScript, TSX, HTML, CSS, Go, Bash, Java, Kotlin, Markdown, TOML, YAML, and
  JSON
- Language servers: diagnostics, completion, hover, signature help, goto,
  references, rename, code actions, formatting, and symbol pickers
- Word completion from every open buffer, including the explorer, with no
  language server and no trigger key required
- Structural Tree-sitter selection, text objects, outlines, syntax-aware
  indentation, and pane-local code folding
- Multiple selections with Helix-style multi-cursor commands
- Counts, named registers, and recordable keyboard macros
- Rope-backed buffers with transactional, single-step multi-cursor undo
- Configurable Unicode-aware soft wrapping and visual-line movement
- Soft-wrap continuation arrows in the line-number gutter
- Asynchronous Git status, gutter marks, changed-file views, file/hunk/safe
  selected-line staging, commit, pull/push, branches, worktrees, history,
  blame, stashes, and diffs
- Side-by-side comparison of any two buffers, aligned line for line and
  scrolled together
- Registry-driven navigation, editing, search, and view-specific commands
- Per-pane jumplists with reversible cross-buffer navigation
- Which-key-style hints for every registered command family, plus exact
  completed-command descriptions and results on the interaction line
- Arbitrarily nested vertical and horizontal splits with recency-aware directional focus
- Shared buffers between split panes
- Oil-style editable directory explorer plus a unified project finder
- Multicursor search over the buffer or a selection, with literal and regular-expression flavours
- Filterable open-buffer picker and workspace-wide text search
- Editable directory buffers with explicit filesystem plan confirmation
- Built-in and user-defined themes, browsable and configurable under `:theme`
- A registry-backed settings browser with previews and atomic YAML updates
- A centred, editable writing viewport toggled with `:zen`, and a plain
  maximized pane toggled with `:fullscreen`
- A bounded, searchable notification center under `:notifications` / `:not`
- Terminal panes running any interactive program — a shell, `htop`, `vim`, a
  coding agent — with scrollback, modal navigation over it, and a command that
  sends a buffer's selection to one as a single paste (Unix only)
- Binary files handed to a program you name, with recent choices remembered
- YAML configuration
- Differential, flicker-free terminal rendering through Ratatui
- Optional `--persistent` local workspace persistence for unsaved editor,
  language-service, and live terminal-session state
- `$EDITOR`-compatible `--wait` requests with revision-safe local edits
- Unicode-aware buffer positions and terminal widths
- Fold-aware mouse cursor placement, drag selection, wheel scrolling, and pane resize
- Undo/redo, yank/paste, dirty-buffer protection, and save-as
- Operating-system clipboard yank and paste

This is intentionally a compact editor, not a complete Helix clone. Some Helix
behavior is deliberately absent and some of it deliberately differs; the
[key bindings](#key-bindings) section records what each binding does here.

### Syntax highlighting

Tree-sitter grammars are compiled into the binary, so highlighting needs no
network access, no grammar directory, and no runtime library loading. Adding a
language means adding a dependency and a row to `src/syntax/grammars.rs`.
Language detection checks an exact filename, then a case-insensitive
extension, then a bounded first-line shebang; Bash currently recognizes
`.bashrc`, `.bash_profile`, `sh`/`bash`/`ebuild`/`eclass` extensions, and
`sh`/`bash`/`dash` interpreters.

Kotlin support recognizes `.kt` and `.kts`, including Kotlin 2 multi-dollar
strings and guarded `when` branches. The pinned grammar does not yet model
context receivers: it parses `context(Logger)` as a separate call before the
following function, so a function text object starts at `fun` and deliberately
does not include the context-receiver prefix.

Files above 128 KB are highlighted without language injection, so embedded
languages such as Markdown fenced code blocks are only highlighted in smaller
files. Markdown's inline grammar is also an injection: headings, lists, quotes,
and other block structure keep their colours above the limit, while emphasis,
strong text, links, and inline code use the ordinary foreground. The injection
query still runs across the whole document on every edit, so this separate
fidelity limit remains in place.

Incremental reparsing runs on a background worker and edits only enqueue the
latest text revision. If typing outruns the parser, one pending request is
replaced rather than growing a backlog. Until the completed tree is drained
between frames, Runyte retains the previous tree and translates its
viewport-scoped highlight spans through the pending edits, so colours remain
visible. Structural features such as outline, folds, matching brackets, text
objects, and structural selection return no result during that interval rather
than querying stale offsets. A completed tree is applied only if both its
syntax base and target text revision still match the live document.

There is no separate line or byte refusal for syntax highlighting. Documents
past the former 200,000-line and 8 MB limits are parsed; a slow parse delays the
new tree, not the keystroke that requested it.

### Language servers

`rust-analyzer` is configured out of the box. For another language, install
the server executable so that it is on `PATH`, then add it directly below
`lsp` in the configuration file. A server that is not installed, that crashes,
or that answers slowly costs you the language features and nothing else — the
editor never waits on one, and diagnostics from a server that stops are
dropped rather than left behind as stale claims about the code.

```yaml
lsp:
  enable: true
  markdown:
    command: marksman
    args: ["server"]
```

`command` is an executable name or absolute path. `args` is an optional YAML
list passed to that executable; Runyte starts it directly rather than through
a shell. Server definitions currently remain a YAML-only setting, while
`Space o o` can enable or disable LSP as a whole. After adding or changing a
definition, exit and reopen standalone Runyte. A persistent session retains
the configuration its host loaded, so restart it with
`runyte --session-restart [WORKSPACE]`; include the same `--config PATH` when
the host used a non-default configuration. `:lsp-restart` restarts a server
from the configuration already loaded by the running editor; it does not
reread YAML.

Servers are keyed by the language names in `src/syntax/grammars.rs`, so a
buffer's language is the same question for highlighting and for LSP. The
available keys are `rust`, `python`, `swift`, `c`, `cpp`, `javascript`,
`typescript`, `tsx`, `html`, `css`, `go`, `bash`, `java`, `kotlin`, `json`,
`toml`, `yaml`, and `markdown`. Other keys below `lsp` are rejected, apart from
the reserved `enable` setting and legacy `servers` wrapper. Use `:lsp-status`
to see servers that have started or failed, `:lsp-restart <language>` to bring
back a loaded server that stopped, and `:service-health` to see whether the
active document has a configured and attached server. A process-launch error
appears in `:lsp-status` after the first start attempt and in the notification
center. `:help lsp` keeps this setup sequence inside the editor.

The older `lsp.servers.<language>` shape remains accepted for existing files,
but `servers` carried no separate behavior and is now only a compatibility
wrapper. New configurations should use `lsp.<language>` as above. Copy-ready
examples for the servers covered by Runyte's real-server compatibility tests
live in [docs/lsp/](lsp/README.md).

Runyte reads what a server actually advertised during its handshake and never
sends a request for a capability it did not claim: hover, completion,
signature help, goto (definition, declaration, type definition,
implementation), references, document and workspace symbols, rename, code
actions, and formatting are each gated independently. Asking for one the
server never advertised reports it as unavailable on the interaction line
rather than as an error, and is not retained as a notification, since it is
expected of a server rather than a fault in it — this also means typing near
a trigger character a server does not support costs no round trip and adds no
notification. A `Method not found` from a server that did advertise the
capability is a protocol violation and is still reported and retained as an
ordinary ERROR.

The same handshake decides *when* completion and signature help are asked
for. A server advertises its own trigger characters — clangd asks for
completions after `/` and `"`, Pyright after `[` and `"`, rust-analyzer after
`'` — and Runyte asks on those rather than on a set fixed for one language. A
server that advertises the capability but names no characters gets Runyte's
own defaults, `.` and `:` for completion and `(` and `,` for signature help.
Signature help follows the same rule, and the lists differ widely: clangd asks
to be consulted on seven delimiters, gopls on two, and sourcekit-lsp and
Marksman do not offer signature help at all. Some servers name the closing
`)`, so
the inner `)` of `f(g(a), b)` asks again there instead of dismissing the popup
— clangd and typescript-language-server then answer with the enclosing
signature, while Pyright names `)` and answers nothing, which closes the popup
as before. A server may instead name retrigger characters, active only while a
popup is showing; Runyte advertises the `contextSupport` that entitles it to
honour them and tells the server which character asked and whether a popup was
already open. A server that names `)` neither way has the popup closed
locally, as before. `tests/lsp_real_servers.rs` records what each server in
the compatibility matrix does.

Real-server compatibility is covered by an opt-in Docker matrix for Python
(Pyright), Swift (SourceKit-LSP), C and C++ (clangd), JavaScript
(typescript-language-server), Go (gopls), Rust (rust-analyzer), and Markdown
(Marksman):

```sh
tests/lsp/run.sh
```

The image pins every server toolchain. Each test uses Runyte's production
stdio transport for the complete LSP handshake and a disposable project.
After the initial symbols and definition checks, it sends an incremental
unsaved change containing non-ASCII text and resolves a definition that exists
only in that changed document. The fixtures then exercise the completion,
hover, signature-help, references, rename, formatting, code-action, and
diagnostic capabilities that each pinned server meaningfully implements; the
per-server declarations in `tests/lsp_real_servers.rs` explicitly distinguish
tested, advertised-only, and unsupported features. Rust, Go, and JavaScript
also resolve definitions across files. Diagnostic fixtures verify both the
reported range and the publication that clears it after repair. The matrix is
deliberately ignored by ordinary `cargo test`; building its image downloads
several large toolchains and running it starts eight real server processes.

### Workspaces and modes

A workspace is one project directory and its editor scope. It exists in both
Runyte modes. **Standalone** keeps the workspace's live editor state in the TUI
process and is the default. **Persistent** starts a persistent session: a local
host and retained editor state associated with that workspace. Open and
unsaved buffers, selections, registers, syntax state, diagnostics, Git
projections, language-server processes, and terminal sessions then remain
alive after the TUI detaches. Persistent sessions are currently available only
on Unix.

Initialize a specific non-Git directory as a workspace with:

```sh
runyte --init /path/to/project   # or runyte -i /path/to/project
```

This creates the configured workspace state directory (`.runyte/` by default)
when it is absent, then opens that exact project directory. If the state
directory already exists, Runyte uses it without resetting or removing
anything. Explicit initialization selects the named directory even when an
ancestor has its own workspace state directory.

Attach to the current project's persistent session with:

```sh
runyte --persistent   # or runyte -a, for attach
```

`workspace.mode: persistent` makes a bare `runyte` do the same; `--standalone`
overrides that setting. Target-bearing invocations remain standalone so their
relative paths and `+LINE[:COLUMN]` positions retain ordinary launch semantics.
`--persistent` reads its argument as a workspace rather than a file; `--wait` is
how a file reaches a persistent session.

`runyte --persistent WORKSPACE` (or `runyte -a WORKSPACE`) attaches to a named
session from any directory, using the same selector the lifecycle commands
accept. A session that is not running is started first, exactly as a bare
attachment starts the current project's. An existing directory the catalog
does not know names that exact directory: Runyte creates its configured
workspace state directory (`.runyte/` by default) when necessary, then starts
its persistent session. A bare explicit `runyte -a` does the same for the
current directory when no workspace is discoverable there. `--init` remains
available for explicitly initializing and opening a standalone workspace.

Persistent mode uses a local host process that owns the workspace state and a
client TUI that displays it. `--persistent` starts the host when necessary and
connects the TUI. `--serve` runs the host in the foreground instead, for direct
supervision or diagnostics; it is also the mechanism Runyte uses when starting
or restarting a persistent session.

`:quit` closes the active pane; from the last pane it stops a clean persistent
session and disconnects the TUI. `:quit-all` requests the same shutdown
regardless of pane count. Quit refuses unsaved buffers and live terminal
children; a `!` form may discard unsaved buffers but still never terminates a
terminal. `:detach` is the explicit leave-it-running operation: it immediately
disconnects the TUI while retaining every pane, buffer, unsaved edit, and
terminal in the host. It is available only in persistent mode and needs no
force form because it discards nothing. Use `runyte --session-stop` to stop a
different persistent session.
Only one interactive TUI may connect at a time; separate control connections
may still manage the host.

`runyte --session-list` (or `runyte -l`) lists running and recently visited
persistent sessions with `ID`, `NAME`, `DIRECTORY`, `STATE`, `UNSAVED`, `TERMINALS`,
`WAITING`, and `TUI` columns,
most recently visited first.
`STATE` is `running`, `stopped`, or `running (protocol N)` for a host left over
from another version of Runyte. Such a host still holds the workspace, so
nothing can attach to it or open a file through it, and its unsaved-buffer
counts are unknown. A normal stop never kills an incompatible host because it
may own live terminals or unsaved buffers. Use a compatible client, or make
the loss explicit with `runyte --session-stop --force`.
IDs are stable hashes of canonical project-directory identities, listed
abbreviated to the six characters they usually need. A listing lengthens them
only if two of its own rows would otherwise read the same, so the ID a row
shows is always a selector that resolves to that row. A project
is named from its workspace directory when it is first recorded. If that name
is already present, Runyte appends the first available numeric suffix, starting
with `-2`; for example, three `runyte` directories become `runyte`, `runyte-2`,
and `runyte-3`. Existing unnamed history entries receive defaults on the next
session listing. Rename a persistent session, running or stopped, with
`runyte --session-rename WORKSPACE NAME`. A running session is renamed through
its host; a stopped one is renamed in the visited history it is listed from.
Session names persist across restarts and must be unique among running
sessions. When a name is entered, surrounding spaces are trimmed and spaces
between words become `-`, so `  release candidate  ` is stored as
`release-candidate`. Default names derived from directories follow the same
rule. `WORKSPACE` may be the abbreviated
ID a listing shows, any other unambiguous ID prefix, the full ID, its exact
session name, or its project directory.
The same selector works for attachment and the lifecycle commands:

```sh
runyte --persistent [WORKSPACE]         # or runyte -a [WORKSPACE]
runyte --session-start [WORKSPACE]
runyte --session-stop [WORKSPACE]       # or runyte -s [WORKSPACE]
runyte --session-restart [WORKSPACE]
runyte --session-stop-all
runyte --session-clear-all
```

Omitting `WORKSPACE` from attach, start, stop, or restart selects the project
found from the current directory. Start is idempotent and leaves the session
detached. Restart starts a detached replacement and retains its name. Stop and
restart refuse while the host owns unsaved buffers, pending `--wait`
requests, or live terminal children. Add `--force` to discard that protected
state; the refusal names each count first. Unsaved buffers never count the
scratch buffer: it has no path, so nothing about it could be saved in place. A
scratchpad someone typed into is therefore never what keeps a workspace alive,
and stopping or retiring the host discards it. A restart does not
retain clean buffers or other in-memory editor state. When a host used a
non-default configuration, pass the same `--config PATH` while restarting it.
`--session-stop-all` applies the same protected-state checks to every running
session and continues after refusals so unrelated clean hosts still stop.
Add `--force` to make the protected-state loss explicit for every host.
`--session-clear-all` removes every stopped row from recent history after
rechecking the inventory; running sessions and project directories are left
alone.

Endpoint metadata and the Unix-domain socket prefer a valid owner-only
`$XDG_RUNTIME_DIR`, with the workspace state root as the fallback, and use
owner-only permissions. A private user-wide cache registry makes both endpoint
locations listable, while XDG-backed hosts also publish a runtime copy so a
missing or unusable cache does not prevent discovery. Dead registrations are
removed while listing, and stale sockets are recovered when a new host starts.
Persistent-session names live under the configured workspace state root,
normally `.runyte/host-names/`. Detached startup carries the already-resolved
workspace identity internally, so the child never rediscovers a project its
parent already resolved.

For tools that need an editor process to stay open, configure
`runyte --wait`. One invocation may name several files and returns success only
after every requested buffer is explicitly closed or completed. `:wbc` writes
and closes the requested buffer without changing the pane layout. An activated
wait buffer enters Normal mode, including when an existing persistent session
was in Insert mode, so `:` commands are immediately available. It reuses a
matching buffer in an existing host. A clean buffer left by an earlier
completed wait is refreshed from disk first; unsaved text and buffers still
owned by a pending wait are never replaced. If that host already has a TUI,
the file appears there while the invoking process waits; if the TUI detaches
before completion, the invoking terminal takes over. If no host exists, Runyte
starts one and attaches the invoking terminal so the request is never invisible.
The command used to complete the buffer owns what happens next: `:wbc` leaves
the host and unrelated buffers running, while `:wq` applies `:q` after writing
an ordinary file and therefore stops a clean persistent session from its last
pane. Dirty buffers retain the ordinary save/discard protection. Detaching the
wait-owned TUI cancels the request, and explicit cancellation or host failure
exits nonzero.

For example, `git config core.editor 'runyte --wait'` gives Git commit and
rebase message files this lifecycle. Persistent hosting and `--wait` currently
use the private, versioned Unix local protocol; it is a bundled-client contract,
not a public automation API.

`Space Space` or `:session-list` (`:sl`) opens the session manager: a
filterable list of running and recently visited persistent sessions, most
recently visited first, with the current session at its head. Enter attaches to
the selected workspace's session in the same TUI and starts it when necessary.
A stopped session keeps its place in that order but is drawn in the theme's
dimmed text colour, the one a command prompt grays the panes behind it with, so
the running hosts stand out without any row being hidden or moved.
`Space Space` is the complete binding,
not a prefix with subcommands. A third `Space` closes the manager, just like
Escape or `Ctrl-c`. Tab opens one manager menu listing only the
actions the selected row's own state can answer: a running row offers Open,
Rename, Renumber, Close, and Force close, and a stopped row offers Open,
Rename, Renumber, and Forget. Open is identical to Enter. Close stops the host
and leaves the workspace listed as a stopped row; nothing below the session is
touched, because a session is the only level that means nothing on its own.

Each row reads as five columns, padded to the widest value in the list so they
line up down it:

```text
  2 * runyte-dev              dev               ~/code/runyte-dev               0min ago
  1   main                    main              ~/code/runyte                   3h ago
  4   runyte.github.io        main              ~/code/runyte.github.io         5days ago
  3   Brain                   -                 ~/Brain                         12days ago
  5   runyte-enh-render-space enh/render-space  ~/code/runyte-enh-render-space  1min ago
```

The number is the digit that attaches to the row, then the session name, then
the checked-out branch, the workspace directory, and the last-active age. The
directory is the widest identity column, so a path under your home directory
is written with `~`; the preview keeps the full path. A workspace that is not
a Git working tree has no branch, so that column holds `-`. The branch is read
from the workspace directory itself rather than answered by a host, so a
stopped session states its branch exactly as a running one does.

Activity uses one short unit at a time: minutes below an hour, hours below a
day, then days, written as `5min ago`, `3h ago`, or `5days ago`. Partial units
round up, including across a boundary, so 59 minutes and one second reads
`1h ago`. The current session reads `0min ago`; leaving or switching away
records the end of that visit, and elapsed values continue advancing while the
manager remains open. When a row is too wide beside the preview, Runyte clips
its middle identity columns while preserving the final activity column. A
history entry written by an older Runyte has no timestamp and reads `-` until
that workspace is visited again.

The selected session's preview, shown in the picker's right column and toggled
with `Ctrl-t`, states the session as a fixed set of fields:

```text
Active: 0min ago
Status      running
Panes       2
Terminals   2 (1 exited)
Buffers     9
Unsaved     0
Waiting     0
Attached    yes
Branch      enh/render-space
Directory   /home/me/code/runyte-enh-render-space
Worktree    yes
Repo        git@github.com:me/runyte.git
```

Every row answers the same questions in the same order, so two sessions are
compared by reading down one place rather than across two sentences. A value
nothing can answer reads `-`, which is deliberately not the same as `0`. The
pane count comes from a bounded, read-only control request made for the selected
row alone, which never becomes a second interactive attachment; it reads `…`
while that request is in flight and `-` for a stopped session or a host using
another protocol version. Nothing in the preview is persisted for later
listings. `Worktree` is `yes` only for a linked Git worktree; it is `no` for a
repository's main checkout and for a directory that is not a Git repository.
Only a checkout that shares a repository with others counts as a worktree: a
`.git` file alone does not make one, since a submodule and a repository created
with `--separate-git-dir` both have one for their own main checkout.

The manager does not show the contents of the session's buffers and terminals.
It did, and at this width a snippet of a pane is neither readable as text nor
useful as identity.

Sessions carry a number from `1` to `9`, shown in the manager's first column.
Pressing that digit in the manager attaches to its session directly, so
`Space Space 1` reaches the first session as one gesture. The digit is a
shortcut only while the manager's filter is empty: Runyte's default names are
`runyte`, `runyte-2`, `runyte-3`, and project paths routinely contain digits,
so once anything has been typed a digit is ordinary filter text. Clearing the
filter, with Delete or by backspacing to empty, arms the shortcut again. A
workspace whose name or path begins with a digit therefore cannot be filtered
by that first character; type a later part of the name instead.

Numbers are assigned in order of creation, when a workspace is first recorded,
and stay with it as the list reorders around them, so the digit does not move
between two visits. A catalog written before Runyte numbered sessions has no
creation order left to recover, and is numbered most-recently-visited first on
the next listing. Only nine sessions are numbered; a tenth is reached by name
or path, and inherits a number when an earlier one is forgotten. Renumber in
the manager menu opens an empty prompt ready for one digit and sets the shortcut
for the selected session; an empty answer takes its number away. Giving a
session a number another one holds swaps the two, so both keep a shortcut.
A standalone workspace owns no persistent host, so the whole `session`
namespace is inert there rather than a set of commands that each refuse.
`Space Space` greys out in the key-hint popup and `:session-list`,
`:session-attach`, `:session-start`, `:session-stop`, and `:session-rename`
grey out in the command palette, exactly as `Space l` and `Space x` do without
a language server or a parser. Invoking one anyway answers
`needs workspace.mode: persistent`.

A running row whose bounded health request does not answer is marked
`health unavailable`, and its terminals, buffers, unsaved buffers, waits, and
attached-TUI state all read `-`: they are unknown, so absence of a count on that
row is not evidence that it is safe to stop. A confirmed zero is shown as `0`
instead, and the unsaved count is the host's own answer, so a healthy running
row showing `Unsaved 0` is one the same host will agree to stop. Retained
terminal screens whose children have exited are counted beside the live ones as
`2 (1 exited)`, since they are not live state.

A row whose project directory has gone while its host is still running says
`missing directory` in its status. It keeps its number and its place so it can
be found and closed; its history record survives the directory's absence for the
same reason, and a stopped session with nothing left to open drops out of the
listing without giving its digit away.
Forget removes only the visited-history record behind a stopped row: nothing in
the project is touched, and naming the directory again starts a host there and
lists it once more.
`:session-attach WORKSPACE` (alias `:attach`) attaches directly and starts a
stopped session. `WORKSPACE` may also be any existing directory; Runyte makes
that exact directory a workspace when necessary before starting its persistent
session.
`:session-start [WORKSPACE]` warms one in the background,
`:session-stop [WORKSPACE]` stops one without switching, and
`:session-rename WORKSPACE NAME` changes its persistent name.
Stopping refuses while the target owns protected buffers, waiters, or live
terminal children; switching away remains safe because the old host retains
them.

Recently visited workspaces are recorded in Runyte's per-user cache, not in
the runtime registry, so stopped projects remain listed across logout. A clean
host with no attached client, outstanding `--wait` request, or live terminal
child retires after
`workspace.idle_retirement_minutes` (1440 by default); zero disables
retirement. `--session-list` prints the same running/stopped inventory.

### Git

When the project is a Git working tree, the status line carries the branch and
what is outstanding — `main ↑1 +2 ~3 -1 ?4 !1` reads as one commit ahead of the
upstream with two added, three modified, one deleted, four untracked, and one
conflicted file. Each file is counted once, under the most consequential thing
that happened to it.

Tracked files also get change symbols in the gutter between the line number
and its separator. `+` marks an added line, `~` a modified line, and `-` a
place where lines were removed, against the surviving row that closed over the
gap. A symbol appears only on the logical line's first screen row; soft-wrap
continuations use that cell for their `↪` arrow instead. Fold triangles share
the cell too; only when a folded anchor is itself changed does the gutter add a
second indicator cell so both `▸` and the change symbol remain visible. Marks
are measured against the **index**, not `HEAD`, so staging a change makes their
symbols go away.

The comparison happens inside the editor. Git is asked for a file's staged text
once, when the file is opened and after Git changes; everything after that is
diffed in memory, so marks keep up with typing without running a process per
keystroke. The asynchronous refresh notices commits, index changes, and branch
switches made elsewhere every five seconds by default. `:git-refresh` remains
the immediate manual reconciliation.

A refresh rewrites a Git view's text and moves the cursor to the nearest row
that survived, so it waits while you are working. It is skipped until you have
paused for one refresh interval, while a prompt is open — including the `/` and
`s` search queries — and while a Git view holds a deliberate selection
such as the matches `s` leaves behind. Nothing is dropped: the refresh runs as
soon as you stop, the prompt closes, or the selection collapses back to a
cursor, and `:git-refresh` still reconciles immediately. A selection in an
ordinary file does not defer anything, because a refresh reconciles its gutter
rather than its text.

When a refresh does land, the cursor keeps its row and its column. The row is
matched by identity — the commit, path, branch, or hunk it was on — and falls
back to the nearest surviving position, with the column clamped to the end of
whatever row it lands on.

`Space g b` opens the local branch list. Enter checks out the selected branch,
and `Tab n` creates a branch at the selected row and switches to it. Both are
refused while the working tree, index, or an open file buffer has uncommitted
changes. If this workspace owns any live terminal session — visible, hidden, or
shown in another pane — Runyte instead asks for the exact target branch name.
Accepting acknowledges that the terminal job keeps its working directory while
Git replaces files beneath it; Escape or Ctrl-c leaves the checkout unchanged.
Exited terminal sessions need no confirmation. Opening another worktree with
`Space g w` remains a workspace attachment switch rather than an in-place
checkout and does not use this confirmation.

`Space g d` opens the active file's unstaged patch. `Space g D` opens the same
comparison as two complete, aligned file versions: the index on the left and
the working tree on the right. In the changed-file list both commands follow
the selected row, so a staged row compares `HEAD` with the index instead. A
missing side of an added or removed file is shown entirely as hatched filler.
The changed-file list's Tab
menu stages and unstages selected files, stages all outstanding files, and
opens everything staged for the next commit. These operations read files on
disk rather than buffers, which is the one place the gutter and the diff views
can disagree — the diff header says so when a buffer has unsaved changes.

Files with no unconflicted entry in the index — untracked, mid-merge, or
binary — show no change column at all rather than a column claiming every line
is new. A project outside Git, or a machine with no `git` on `PATH`, simply has
no gutter and no branch in the status line.

### Terminals

`:terminal` runs a program in the active pane. With no argument it runs
`$SHELL`; `:terminal htop` runs a command line, split the way a shell splits
it. `:term` and `:t` are the same command. `Space t n` is the same command on
a key, and the pane's own buffer stays where it is — leaving the terminal
shows it again.

Bare `:terminal` keeps using the editor working directory. The explicit
variants are `:terminal-file-directory [command]`,
`:terminal-directory-root [command]`, and
`:terminal-selected-directory [command]`. `:terminal-session-directory
<id|name>` starts a shell at another terminal's last safe directory. Shells
may update that value with a bounded local OSC 7 `file:` URL; remote hosts,
control characters, non-file schemes, and paths that are not existing absolute
directories are rejected.

In **Insert mode keys go to the program except for Runyte's terminal exit and
window prefix**. `Ctrl-\` leaves input in live Normal mode. `Ctrl-w` instead
begins the registered window namespace: `Ctrl-w h/j/k/l` and their
control/arrow aliases move to another pane immediately. Every focus route,
including these keys, pane cycling, and a mouse click, activates a live Normal
terminal destination in Insert mode. A terminal that already owns a captured
review stays in Normal/review mode until `i`, `a`, `o`, or another terminal
insert key returns it to the live screen; a document reached from Terminal
Insert starts in Normal. `Ctrl-w v/s` and their control-key aliases split the
only pane while leaving the child live in the original pane. `Ctrl-w f/z`
toggle full-screen and zen presentation from terminal Insert, live Normal,
Normal/review, and Select without changing that mode. Bare `Ctrl-w` likewise
keeps the current mode. Window actions never capture or discard a terminal
review snapshot. Canceling the prefix leaves the terminal in Insert mode.
`Escape`, `Ctrl-c`, `Ctrl-o`, `Space`, and ordinary keys still reach the child
unchanged.

Turning on `editor.fast_pane_keys` adds `Ctrl-h/j/k/l` to that short list of
keys the terminal does not own, so a pane move costs one keystroke instead of
two. The child then never sees those four, which is the trade: `Ctrl-h` is
backspace and `Ctrl-l` clears the screen in most shells. `Ctrl-w` keeps
working either way.

The exit is staged: the first `Ctrl-\` changes INSERT to live NORMAL without
freezing output, and the second captures the terminal's immutable review
snapshot. `i` returns either Normal state to terminal input. Runyte requests
unambiguous Ctrl-key reports on macOS without requesting repeat and release
events there. A terminal that does not implement that protocol keeps using
legacy control bytes, where the same physical `Ctrl-\` key arrives as
`Ctrl-4`; both spellings work.

**Normal/review mode navigates and copies. It does not edit.** The cells on screen are
a picture of the program's text rather than the text itself: the program owns
its own input area, and an edit made to those cells would be an edit to a
picture. A second `Ctrl-\`, or the first review command used from live Normal,
captures the bounded output as an immutable review snapshot while new child
output continues behind it. Ordinary
character, word, line, vertical, paragraph, and page motions move a visible
review caret and keep it inside the configured viewport margin; `v` enters
Select mode, where those motions extend the selection inclusively through both
the character where it began and the character they land on, in either
direction. `Escape` cancels a selection made either way. `f`/`F`/`t`/`T` find
characters in the snapshot, and `gw` labels its visible words and jumps to the
chosen one. `%` selects all retained review text. `s` searches it
case-insensitively, `/` searches it with a regular
expression, and `n`/`N` or `)`/`(` move through stable highlighted matches. `y` copies the caret
character or review selection to the unnamed Runyte register and `Space c y`
copies it to the system clipboard. `p` or `P` discards review, sends text from
Runyte's selected register to the live child, and enters Insert mode so the
next key (usually Enter) reaches the child. `Space c p` / `Space c P` send the
system clipboard and remain in Normal mode. Both paste routes write at the
child's real cursor after returning to the live screen, never into captured
output. `u` takes the last such paste back by
sending the child one delete per character sent, which a shell at a prompt
answers by erasing exactly the paste. It is offered only while that paste is
still the last input the child received — any key typed into the terminal ends
it — and it is refused for a paste that ended a line the child has already run,
since what ran is the child's. `i` discards review and types again. The
title marks review state and newer output, and a reviewed terminal's text is
grayed out, so a frozen still image never looks like a child that is still
printing. Moving to another pane does not start review; returning to a terminal
that is already under review does not discard its snapshot. A live terminal
keeps its own colours. Alternate-screen review contains only its captured
visible screen.

For real Runyte editing over what a terminal printed, `Space t y`
(`:terminal-output`) copies the session into an ordinary read-only buffer.
That is real text: search, multiple selections, `n`/`N`, and yank all work on
it. `Ctrl-o` or `Alt-o` returns to the terminal, and the corresponding
`Ctrl-i` or `Alt-i` returns to the generated output.

Composing goes the other way. Write the text in an ordinary buffer with every
editing command available — multiple cursors above all — and `Space t s`
(`:terminal-send [id|name]`) sends the selection to a terminal as one bracketed paste,
or the whole buffer when nothing is selected. This is the only way modal
editing can reach a program that owns its own input area, and it is what makes
long prompts for a coding agent worth writing in the editor.

A session outlives the pane showing it. `Space t q` shows the pane's buffer
again and leaves the program running; so does opening a file in that pane or
closing the split. `Space t t` (`:terminals`) lists stable sessions with their
user name, child-title detail, safe directory, unread output,
and bell activity; picking one shows it here. If that session is already
visible in another pane, it moves here and the old pane reveals its underlying
buffer—one PTY is never resized by two visible panes. `Space t r`
(`:terminal-rename <name>`) names the active session,
`:terminal-show <id|name>` targets one deterministically,
and terminal termination stays explicit: type `exit` in the child, or choose
Close from the terminal manager's Tab menu. Duplicate names are refused as
ambiguous and numeric IDs never depend on picker order. Tab in the manager
offers Show, Rename, Close, and Create; closing a hidden live process requires
a second Enter. Neither `:close[!]` nor any `:quit…` command terminates a
terminal.

Terminal sessions use the `[terminal] <name>` prefix in pane and manager
titles. The active pane adds `[insert]` while keys go to the child; NORMAL is
unmarked, because the mode line already says it and the title's job here is to
answer whether typing reaches the child. The name itself is user-assigned when
present, otherwise whatever the program calls itself — the title a shell sets
from its prompt, or the program's own name until it sets one. Scrolled back
into history it also carries `↑` and how far. When the child exits, its session
disappears from `:terminals`, and a pane showing it reveals its most recently
used buffer (or a scratch buffer) without closing the pane. Every quit spelling,
including its `!` form, refuses while any terminal is running and points to
`:terminals` in both standalone and persistent modes. `:detach` leaves
persistent terminal children running without signalling them. Sessions survive
normal detach, client failure, workspace switching, and reattachment for the
lifetime of that same workspace-host process. They do not survive force-stop,
host replacement or crash, logout/reboot, or machine failure.

The emulator is Runyte's own, like the fuzzy scorer, the picker, and the diff.
It implements what an interactive program on a pty needs: colour including the
256-colour palette and true colour, the usual attributes, scroll regions,
insert and delete, the alternate screen, bracketed paste, application cursor
keys, cursor-position reports, and window titles. Scrollback is bounded at five
thousand lines per session and by a measured 64 MiB retained-cell/review
payload budget per workspace. Noisy sessions use independent bounded queues
and round-robin byte/message budgets; PTY input is bounded and chunked too.
Terminal and review output is never written to disk. Attached output frames are
coalesced to a bounded cadence and ordinary repaint uses revisioned changed-row
damage, with complete-frame resynchronization on a stale base. Rapid identical
wheel reports are likewise coalesced into bounded requests while preserving
their scroll distance and their order relative to clicks and keyboard input.
The alternate screen has no history by construction, so while `htop` or `vim`
is running there is nothing behind the visible screen to scroll to or copy.
Primary-screen inline TUIs may keep a composer or status area fixed while
scrolling completed output through a top-anchored region; those completed rows
remain ordinary scrollback, including Codex output when it runs in inline mode.

Known limitations. Terminals are Unix only: Windows needs ConPTY, which is a
second implementation of the hardest part, and
`context/issues/windows_support.md` already records that Runyte disables a
feature there rather than shipping an unsound one. SGR mouse reporting is
forwarded inside terminal pane bodies when a child requests it; borders remain
Runyte's, and the wheel scrolls review history when the child has not requested
the pointer. A cell retains up to three combining marks without consuming
columns; additional marks are deliberately bounded. Inline images — kitty
graphics, sixel — are not passed
through. Resizing does not reflow wrapped lines, because emulators disagree
about what a resized wrapped line should become and a wrong guess corrupts a
live full-screen program worse than a truncated one. Read-only `OSC 10;?` and
`OSC 11;?` queries report the current theme's default foreground and
background, so light- and dark-aware programs choose colours that fit their
pane. A theme colour set to `reset` remains unknown and receives no reply.
Colour-setting and palette queries remain ignored, as does `OSC 52`, so a
program cannot change the terminal palette or write your clipboard unasked.

### Comparing two files

`:diff-this` marks a buffer, and running it again in a second buffer shows the
two side by side. It is spelled `:difft` or `:dt` for short, and `:diff-off`
(`:do`) closes the comparison again. `:diff-this` in the buffer already marked
takes the mark back.

Where the two buffers are does not matter. If the one marked first is not on
screen, the second command splits for it, and the buffer marked first is the
one on the left — the order the two commands were typed. If both are already
in panes, those panes are used as they are.

Corresponding lines sit level. Where one file has lines the other does not, the
other side holds the gap open with a hatched filler row that belongs to no
line: it cannot be clicked, labelled by `goto-word`, or moved onto, in the same
way the blank area past the last line cannot. The two sides therefore scroll
together — whichever pane you are in leads and the other follows the line
facing it, which is not the same line number once the files have drifted apart.

Both files stay editable, and the comparison follows what you type rather than
describing what the files used to say. Changed lines are filled and marked in
the gutter: what only the right side has reads as added, what only the left
side has as removed, and lines that answer to different lines on the other side
as changed. Lines replaced by an unequal number of lines are one change rather
than a deletion stacked on an addition, which is the same folding the Git
gutter does. In a comparison that column shows the comparison rather than the
Git marks, since one column cannot answer both questions at once.

Soft wrap is off while a comparison is open, whatever `editor.soft_wrap` says,
and comes back when it closes: lines are matched whole, so a wrapped line would
take a different number of screen rows on each side and pull the two views out
of step while still being correctly aligned. Collapsed regions are expanded
when a comparison opens for the same reason.

A comparison needs both its panes and both its buffers. Closing either pane, or
pointing one of them at a different buffer, ends it — the view was about those
two files.

### Files changed outside Runyte

Runyte monitors the parent directories of open ordinary files in standalone
and persistent modes, including while a persistent TUI is detached. When a
path no longer agrees with the disk state accepted at open, save, or reload,
every pane showing it and the global status line show `[STALE]`; the buffer
manager marks hidden stale buffers too. This is separate from `[+]`, so a
conflict reads `[+] [STALE]`. Detection preserves text, selections, undo, and
language-server state, and the first observation of each disk revision creates
one retained WARNING notification.

`Space b d` or `:diff-disk` reads the path again and opens that immutable disk
revision as `[disk] path [RO]` on the left of the editable Runyte buffer. The
view uses the ordinary aligned side-by-side comparison and follows later edits
on the right. Deleted, unreadable, binary, and versions over the comparison
size limit have no comparison source and are refused without changing panes.

`Space r` or `:reload` reloads a clean file immediately. For every dirty file,
it first opens a confirmation naming the path and the undo history that will be
discarded; Escape or `Ctrl-c` keeps the buffer. Enter applies only the exact
disk revision reviewed by that confirmation. If the path changes again, the
buffer is retained and the new revision must be reviewed again.

When the in-memory and observed disk text become equal, Runyte adopts the new
disk identity and marks the buffer clean without clearing usable undo history.
A deleted path remains stale but an ordinary save may recreate it. Binary and
unreadable replacements are never admitted into a text buffer. An ordinary
save refuses a known conflicting revision before save hooks can edit the
buffer; `:write!` is the explicit boundary for replacing it and clears
`[STALE]` only after the installed file is verified.

### Directory buffers

Open a directory with `runyte <directory>` or `:open <directory>`. It appears
as an editable buffer with one relative path per line and `/` after
directories. Normal modal editing and multiple selections work unchanged:
rename a line to rename an entry, remove it to delete, add a line to create a
file, add a trailing `/` to create a directory, or change its path to move the
entry. To copy with the normal Helix keys, select one or more entries with `x`,
yank with `y`, navigate or focus the destination explorer, and paste with `p`.
Use `d` instead of `y` to cut and move the selection. This works after
navigating in the same pane or across split panes. A pasted cut is applied by
writing its destination explorer; Runyte refuses to write the source first
because that would delete the move's source. For a copy in the same directory,
rename the pasted row with normal Helix editing before writing.

Editing and pasting never mutate the filesystem immediately. `:w` or `:write` opens a
plan listing every create, rename, move, copy, and delete. Enter applies it with
deletions sent to the operating-system trash, `P` explicitly opts into
permanent deletion, and Escape cancels. Runyte rejects the whole plan if the
directory's visible entries changed on disk since it was opened or if an entry
the plan will move, copy, or delete changed. Activity inside an unaffected
child directory does not stale the explorer. If an operation fails midway, an
ERROR notification names the failed operation and every operation already
applied.

### UI vocabulary

The complete terminal surface is the **Runyte screen**. Its upper **editor
area** contains one or more **panes**. A pane has a **pane border**, a **pane
title** in its top border, and a **pane body** inside it. The pane body contains
the **gutter** (line numbers, soft-wrap markers, syntax-fold markers, and Git
change marks), optional **content padding** used by aligned generated pages,
and the **buffer viewport** where buffer text is displayed and, for editable
buffers, changed.

Two global rows sit below the editor area. The **global status line** reports
mode, workspace directory, active-buffer state, cursor, Git/LSP state, and
unread notifications. The
**interaction line** below it is reserved for an active prompt or the last
action echo. These names are also the vocabulary used by help and source
documentation, and any future extension surface must inherit them.

Runyte chooses a surface by the lifetime and interaction of the task. A
navigable result is a **buffer** (or a pane-backed filterable list) and keeps
normal movement, selection, search, splits, help, and buffer management. A
**special buffer** is one whose contents Runyte assembles rather than reads as
ordinary file text, such as the explorer, config, Git views, notifications,
help, or the about page. It remains a full buffer while displayed, may be
shared by panes, and may be editable or read-only. Runyte retains the two most
recently active clean special buffers across pane switches; activating a third
retires the least recent one once it is detached. A dirty special buffer
remains available until saved or discarded. A pathless scratch buffer is
ordinary text rather than special. A transient choose-one request is a
**picker overlay**; source-tied assistance is
a **context overlay**; a pending prepared operation is a **confirmation
overlay**; short scalar input uses the interaction line; and richer validated
input uses an **input overlay**. The shape of a box does not determine its
role.

For example, `Space g l` opens a Git-log buffer for ordinary browsing, while
`Space g /` opens a fuzzy commit picker that disappears after one choice. Both
open the same retained special commit-detail buffer when a commit is selected.

The canonical definitions live in
[`context/reference/ui-vocabulary.md`](../context/reference/ui-vocabulary.md).

### Global status line

The leftmost mode label in the global status line uses the current mode's caret
colour — blue for Normal, red for Insert, neon green for Replace (neon magenta
when another mode already uses green), orange for Select, and purple for
Command. The rest of the row keeps the
theme's ordinary background. Its left side
then names the workspace mode and current workspace directory, marking the
active buffer `[+]` when it has unsaved changes, `[STALE]` when its file path
disagrees with the accepted disk baseline, and `[RO]` when it is read-only.
Pane titles carry file and buffer identity. The right carries the cursor and how
far through the file it sits:

```
 NOR │ standalone │ Workspace: /home/me/code/runyte [+]   412:17 · 34% │ 3 sel │ main ~1 │ rust-analyzer 0E 2W
```

`standalone` means the TUI owns the live workspace state in its process;
`persistent` means the state survives in a separate local process. These are
the same two values accepted by `workspace.mode` and selected by the command
line flags above.

When the directory does not fit, Runyte preserves its identifying end and
replaces the beginning with `...`.

`412:17 · 34%` is the cursor's line and column followed by its distance
through the buffer. The first line reads `0%` and the last reads `100%`, and no
line between them does: a cursor one rounding step from either end is reported
as `1%` or `99%` rather than claiming an end it has not reached. A one-line
buffer has no distance to cover and reads `100%`. The percentage follows the
cursor rather than the topmost visible line, so `Z j` and the other view-scroll
keys leave it alone until the cursor itself moves.

The fields after it appear only when they apply: the selection count above one,
the Git branch and outstanding-change counts, the language-server summary, and
unread notification counts such as `E1 W2 I3`. ERROR, WARNING, and INFO counts
use their semantic theme colours. At narrow widths the counts compact to the
highest unread severity plus the total, for example `E4`.
The active theme is not among them; `Space o t` shows and changes it.

## Install and run

Runyte currently supports Linux and macOS. Windows support is planned for a
future release, but current releases should not be considered Windows-supported.

You need Rust 1.88 or newer and a C compiler; the bundled tree-sitter grammars
are built from C source during the install.

```sh
cargo install runyte --locked
runyte README.md
```

`--locked` matters. Without it Cargo discards the dependency versions this
release was built and tested against and re-resolves the whole graph to the
newest compatible ones, which can pull in a crate requiring a newer toolchain
than Runyte itself needs. The install then fails naming a transitive
dependency you have never heard of.

That installs the `runyte` editor executable.

To build from a clone instead:

```sh
./build.sh --release
./target/release/runyte README.md
```

Runyte accepts multiple startup text files and opens them all before entering
the terminal, leaving the first text file active. Put a one-based `+LINE` or
`+LINE:COLUMN` immediately before a file to place its caret when that buffer is
first shown; columns count Unicode characters rather than bytes. For example,
`runyte +12:4 "notes with spaces.md" src/main.rs` opens both files at once.
Repeated spellings of the same resolved path share one buffer, both at startup
and when opened later in standalone or persistent mode. A save-as refuses a
path already owned by another live buffer, including its resolved aliases;
close that buffer before deliberately taking the path over. Use `--` before
paths beginning with `-` or `+`, as in `runyte -- +draft.md -notes.md`.
Binary startup targets still use the interactive external-program prompt;
open binary files one at a time so no explicit target can be silently skipped.
The prompt initially selects the preferred application registered with the
desktop (`xdg-open` on Linux and `open` on macOS). Use Up and Down to select an
application, Enter to open with the selected application, or type another
program. Explicit choices are remembered. Press Tab on a remembered choice to
delete it or make it the default selection for later binary files.

Piped/stdin scratch input (`runyte -`) is deliberately deferred: Crossterm
owns stdin for terminal events in the current standalone process. Before `--`,
a lone `-` therefore exits with an actionable explanation instead of entering
a terminal that cannot reliably distinguish file contents from input events.

Terminals with mouse reporting support can click an editor body to focus it
and place the caret, Shift-click to extend, drag to select, scroll the pane
under the pointer, and drag a shared pane border to resize the split.
Right-clicking any current selection copies all current selections to the
system clipboard, exactly like `Space c y`, without moving or replacing them.
The interaction line reports
`right mouse click (yanked to system clipboard)` after the copy succeeds.
Clicking a live terminal pane focuses it in Insert mode; clicking a reviewed
terminal keeps it in Normal/review mode until a terminal insert key returns to
the live screen. A left-button drag in that immutable review selects terminal
cells and enters Select mode; right-clicking that selection copies it to the
system clipboard by the same rule above. Clicking another pane focuses it in
Normal mode. Dragging on from a document press still creates a selection and
enters Select mode. The
pointer names the character it sits over rather than the boundary before it,
so a drag covers both the character it started on and the one it ended on,
whichever way it ran. Pressing past the end of a line places the caret on that
line's last character, or past it while Insert mode holds the pane, which is
the same rule keyboard motion follows.
Pointer coordinates are resolved through the same fold- and wrap-aware
prepared rows used for rendering, so collapsed lines and wide Unicode glyphs
do not create a second coordinate model. Mouse capture is disabled again on
every terminal exit/error path. Capture also takes ownership of the terminal's
native text selection; set `editor.mouse: false` when native selection is
preferred and restart Runyte. Passive pointer motion never clears key hints or
status and does not schedule a redraw.

Pending compound bindings open a key-hint popup. `Ctrl-n` and `Ctrl-p` scroll
its commands without participating in the pending binding. Up and Down also
scroll unless that arrow completes a binding under the pending prefix;
`Alt-j` and `Alt-k` remain alternatives. Scroll controls appear in the title
when the entries exceed the available space.

`build.sh` wraps `cargo build --bins` and forwards any extra arguments. A local
Cargo installation installs the editor:

```sh
cargo install --path .
```

During development:

```sh
./build.sh
cargo run -- src/main.rs
cargo test
```


Run `runyte --help` for command-line options. Inside the editor, press
<kbd>Space</kbd> then <kbd>?</kbd> for contextual help.
With no file argument, `runyte` opens its read-only about page. `runyte .`
opens the current directory in the explorer, while `runyte file.txt` opens that
file directly.
Pressing a prefix such as <kbd>g</kbd>, <kbd>Space</kbd>, or
<kbd>Ctrl-w</kbd> shows its available continuations automatically. The default
Runyte grammar groups canonical commands under labelled `Space` namespaces;
the popup shows one namespace row at a time, then reveals its generated
registry entries when you enter it. Existing short keys remain fast or
compatibility bindings. A row that names two keys, such as `Space s a, &`,
runs the same command either way, so the namespace teaches the short spelling
rather than hiding it. `Space l` is labelled **Language (LSP)** and `Space x`
is labelled **Syntax (Tree-sitter)**. Either row is dimmed with an unavailable
reason when the active file does not have that service ready. The namespace
remains navigable so its individual commands can explain their availability;
in particular, LSP status and restart can remain usable without an attached
document. The **Git** row at `Space g` follows the same rule and is dimmed when
Git is not installed or the current project is not inside a Git repository.

`:grammar` reports the active Runyte grammar. `helix` remains accepted as a
configuration and command alias for `runyte`.
The former `vim` grammar has been removed; configurations that still set
`editor.grammar: vim` are rejected with an invalid-value error and should use
`runyte` or remove the setting.

## Key bindings

The interaction line teaches completed commands as they are used. A binding
that has no more specific result leaves its exact typed spelling and registry
description, such as `g l (Move to line end)`. Commands with a useful result
keep it, for example `Space e (opened /path/to/dir)` or `:c (closed file)`.
An active prompt temporarily uses the line. Errors, warnings, informational
service output, and other notifications never replace the prompt or action
echo. A failed or unavailable action is echoed as `binding (action · failed:
message)` or `binding (action · unavailable: message)`, carrying the
outcome's own message rather than only the fact that it failed. A message
that would run past the space actually left on the line, or that spans more
than one line, is cut to what fits and shown with a trailing `...`; `:not`
keeps the untruncated, complete text regardless. A prompt still typing on
the line is never cut this way, since its cursor position depends on the
untruncated text.

`:notifications` (alias `:not`) opens `[notifications]`, a single searchable,
read-only buffer containing the retained history newest first. Each entry has
a local `YYYY-MM-DD HH:MM:SS` timestamp, Runyte-assigned `ERROR`, `WARNING`, or
`INFO` severity, source, title, and retained multiline details. Opening it
acknowledges the entries then retained. New notifications are queued without
moving focus and become unread. Consecutive identical notifications coalesce,
update their timestamp and occurrence count, and become unread again.
Producer safety bounds remain explicit: one notification retains at most 1 MiB
and one workspace at most 8 MiB, in addition to the configured entry-count
limit. Git failures retain labelled stdout and stderr within that 1 MiB budget,
enough for thousands of ordinary lines, and append a visible truncation marker
when a hostile hook exceeds it. The rest of an overlong stream is still drained
while the subprocess runs, so retaining less diagnostic text does not itself
break an otherwise successful hook or helper. Successful asynchronous Git
output updates the initiating action echo when it is still current; multiline
output, or output whose echo has already been superseded, is retained as
`INFO`. A mistyped key sequence (`No binding: g z`) is reported the same way on
the interaction line and in the key hints, but is not retained: it says nothing
worth reading back later, and a burst of mistyping would otherwise be the
single largest and only unbounded contributor to the unread count.

`:about` opens a centered, read-only introduction with Runyte's logo, current
version, and a short getting-started guide. It is centered against the pane
rather than against fixed columns in its own text: resize the window or split
the view and the page moves with it, without a character of it being rewritten.
The space around it is drawn, not stored, so searching, scrolling, and clicking
land on the text itself. A page taller than its pane is shown from its first
row and scrolls like any other read-only buffer.

`:tutorial` opens a guided introduction in two ordinary panes: read-only
instructions and disposable scratch text. The first picker asks whether lesson
prose should show Vim-like motion spellings, Helix-like motion spellings, or
both. That preference changes only the spellings displayed by the tutorial;
Runyte's selection behavior and keymap do not change. Lessons cover modes,
selection-first editing with characterwise and `x`/`X` whole-line selections,
search, multiple carets, `Space` discovery, and `Ctrl-w` pane commands. The
hands-on view lessons distinguish scratch, generated, and editable explorer
buffers from terminal pane content: they open the explorer, return through
`Alt-o` buffer history, create an integrated terminal, and explicitly close
that terminal session through its manager. The final lessons cover
`Ctrl-o`/`Ctrl-i` jump history, the standalone/persistent boundary, and point
to `:help` plus `Space ?` in each view for further learning.
Reopening `:tutorial` resumes the live lesson, `:tutorial reset` starts over,
and `:tutorial sessions` opens the persistent-session lesson directly.

`:zen` applies the same presentation boundary to an editable writing view. It
temporarily maximizes the active pane and centers a text viewport up to
`editor.zen_width` cells wide (100 by default); a narrower terminal uses all
available cells. The buffer, selections, undo history, and split tree stay
unchanged, and a second `:zen` restores the exact prior layout. Soft wrapping
continues to follow `editor.soft_wrap`.

`:fullscreen` maximizes the active pane the same way but enforces no width at
all: the pane is an ordinary pane that happens to fill the editor area, with
its text laid out exactly as it would be in a split. `Space w f` toggles it and
`Space w z` toggles Zen, each with the usual `Ctrl-w` compatibility spelling.
The two are one state rather than two, so asking for the other view while one
is showing switches to it instead of stacking a second maximization on top of
the first, and only the view actually showing toggles off. Pane splitting and
closing wait until the maximized view is toggled off so no hidden layout change
can be mistaken for the restored one, and directional focus and pane cycling
are refused for the same reason: while one pane is maximized it is the only
pane keys can reach, so what is on screen and what is being typed into cannot
come apart. The maximized pane's title carries `[zen]` or `[fullscreen]` after
its `[+]` and `[RO]` markers, so the one pane on screen says which view is
hiding the rest of the layout; an ordinary pane carries neither tag.
In the Runyte grammar, `Space ?` opens contextual help for the current buffer
type. `:help` and its `:?` alias instead open the general Runyte manual;
`:help <topic>` opens the same manual at a section such as
`:help regex`, `:help search`, `:help mouse`, `:help git`, or `:help lsp`. Both
kinds of help are ordinary read-only buffers, so they scroll, search, split,
and close with their scoped `q`, `:c`, or `Space b c`. Nothing is truncated to
fit the window, and opening one kind of help does not overwrite the other.

A read-only buffer is marked `[RO]` in the pane title and the status line,
beside the `[+]` that marks unsaved changes, and its help states the same in
full: `Help · RUNYTE · GIT STATUS · Read-only`. The global status line describes the
buffer on screen; the help title describes the buffer type the document is
about, so help for an editable type is not titled read-only even though the
help buffer showing it is.

Pane titles name the buffer kind structurally: ordinary paths are prefixed
with `[file]`, directory paths with `[explorer]`, and virtual views retain
their existing bracketed names such as `[git status]` and `[git branches]`.

Contextual help describes the view it was opened over — one document per
buffer type, not one per mode. NORMAL and SELECT bind the same keys to the same
commands, so a text buffer has a single `TEXT` document that describes both
modes in its prose rather than two whose key tables would be identical.
Sections run from most to least specific:

- **Buffer keys** — direct keys unique to the view and the contextual actions
  its `Tab` menu opens, such as `Tab s` to stage a changed-file row.
- **Where to start** — every prefix that opens the hint popup: `Space`, `g`,
  `z`, `Z`, `m`, and `Ctrl-w`.
- **Direct keys** — everything that acts on the first press, grouped into
  letters and punctuation, `Ctrl` chords, `Alt` chords, and named keys.

`Ctrl-o` and `Ctrl-i` walk every recorded position, including terminal surfaces
and positions within one file. `Alt-o` and `Alt-i` walk the same history but
stop only on a different buffer or terminal surface, so leaving a document you
have read through is one press rather than one per section. Closing a buffer
with `:c` or `Space b c` keeps every pane and returns each one to its own most
recently used live buffer. When none remains, the pane receives a new scratch
buffer.

Every key named there is read from the keymap registry when help is opened, so
it cannot drift from what the keys do. In a read-only view, keys that would
only report a refusal are left out entirely.

In Normal and Select modes, `Tab` asks what can be done with the thing under
the cursor. Git views open a contextual action menu: use arrows or `j`/`k` to
move, Enter to run the selected action, its displayed mnemonic to run it
directly, and Escape to cancel. Each row reads across four aligned columns:
the mnemonic, one word naming the action, whether it acts on the row under
the cursor or on the whole buffer, and a sentence explaining it. The
mnemonics belong only to the open menu, so `s`, `n`, `p`, and the rest keep
their normal meanings in every buffer. In an ordinary language-server buffer,
`Tab` requests code actions and opens the existing code-action picker, or
reports that none are available.

### Editing

The one-key rows in this table are the complete direct-binding inventory for
Normal and Select modes. Common prefixed gestures are included afterward for
context; scoped explorer keys are documented under
[Directory buffers](#directory-buffers), and Insert-mode keys under
[Insert mode](#insert-mode).

| Key | Action |
| --- | --- |
| `h` `j` `k` `l` or arrow keys | Move left, down, up, right |
| `w` / `b` / `e` | Next word / previous word / word end |
| `W` / `B` / `E` | Long-word variants |
| `f` | Find the next typed character in the buffer |
| `F` / `t` / `T` | Find backward, or move until a character forward / backward |
| `Home` / `0`; `End` / `$` | Start / end of line |
| `Ctrl-b` / `Ctrl-f`; `Ctrl-u` / `Ctrl-d` | Page up / down; half-page up / down |
| `PageUp` / `PageDown` | Page up / down |
| `gg` / `ge` or `G` | Start / end of file |
| `gp` / `gP` | Next / previous paragraph |
| `gw` | Dim the view, label nearby words with one key and farther words with two, then type a label to jump |
| `i` / `a` / `I` / `A` | Insert before/after cursor or at line boundary |
| `o` / `O` | Open line below / above |
| `r` / `R` / `~` | Replace once / enter Replace mode / toggle case |
| `v` | Enter Select mode |
| `x` / `X` | Select current line, then extend down / up |
| `%` | Select the entire buffer |
| `C` / `Alt-C` | Add a cursor on the nearest line below / above holding a character at the cursor's column, skipping the ones too short |
| `V` | Add a cursor on the next line, padding short or empty lines with spaces to the same display column |
| `;` / `Alt-;` | Collapse selections / flip their direction |
| `,` / `Space s c` / `Alt-,` | Keep only the primary selection / drop it |
| `)` / `(` | Make the next / previous selection primary |
| `Alt-)` / `Alt-(` | Rotate selection contents forward / backward |
| `&` | Pad until every cursor shares the rightmost display column |
| `_` | Delete trailing whitespace from every selected line; `%` then `_` strips the buffer |
| `Alt-_` | Shrink every selection past the whitespace at its ends, without changing the text |
| `Space p .` | Toggle dim `·`, `→`, and `↵` markers for spaces, tabs, and line endings |
| `d` / `c` | Delete / change selection or cursor character; `d` after transient `x`/`X` cuts whole lines |
| `y` / `p` / `P` | Yank selection or cursor character / paste after / paste before |
| `Y` | Yank every line the selection touches, as whole lines |
| `>` / `<` | Indent / unindent |
| `Ctrl-c` | Comment or uncomment every line the selection touches, using the buffer language's line comment; also bound in Insert mode |
| `u` / `U` | Undo / redo |
| `s` | Search, ignoring case |
| `/` | Search with a regular expression |
| `n` / `N` | Step to the next / previous match |
| `*` | Select every occurrence of the word or selection under the caret |
| `Ctrl-o` / `Ctrl-i` | Jump backward / forward through navigation history |
| `Tab` | Open contextual actions for the selection or row under the caret |
| `Ctrl-s` | Save |
| `:` | Open the command palette |
| `\|` | Shell pipe (reserved but unsupported) |
| `<n>` before a command | Repeat a motion or countable command |
| `<n>gg` / `<n>G` | Go to line `<n>` |
| `"` then a register | Select a named register; uppercase appends and `_` discards |
| `Space m …` | Record, replay, and list macros; see [Macros](#macros) |
| `mm` | Jump to the matching bracket |
| `z…` / `Z…` | View alignment and scrolling |
| `Esc` | Return to Normal mode |

<a id="insert-mode"></a>

### Insert and Replace modes

Insert mode adds text at every caret. Replace mode starts with `R`, collapses
every selection to its active head, and overwrites the character ahead of each
caret as text is entered. A caret already at line end appends instead, and a
newline inserts a line break rather than consuming the existing terminator.
Unicode characters are replaced one for one and CRLF remains one line ending.

Backspace in Replace mode retraces the current overwrite run: overwritten
characters return, while characters appended past line end are removed.
Alt-Backspace and Ctrl-u restore by word and to the beginning of the current
line. Escape or Ctrl-`\` returns to Normal mode, and the complete Replace-mode
session is one undo checkpoint. Lowercase `r` remains the single-character
Normal-mode command and never enters Replace mode.

`Ctrl-c` comments or uncomments every line the selection touches, taking the
marker from the buffer's language: `//` for Rust, C, C++, Go, Java, JavaScript,
TypeScript, TSX, Kotlin, and Swift, and `#` for Python, Bash, TOML, and YAML.
CSS, HTML, JSON, and Markdown have no line comment, so there the key reports
that and changes nothing rather than inventing a marker the language cannot
parse.

The marker goes at the least-indented line in the block, so a nested line keeps
its relative indentation across the round trip, and blank lines are left alone
in both directions. Uncommenting removes the marker and at most one space after
it, so `// x` and `//x` both come back as `x`. A block where only some lines are
commented commutes to fully commented first, which is what makes a second press
always the inverse of the first. When an extensionless script gets its language
solely from a recognized first-line shebang, that row is left unchanged so the
buffer keeps its language and the rest of the selection can round-trip.
`Ctrl-c` is bound in Insert mode as well, where it acts on each caret's own line
— entering Insert mode collapses selections to carets, so a block has to be
chosen in Normal or Select mode.

### Search

Search has two flavours and one behaviour. `s` matches text ignoring case, and
`/` interprets the pattern as a regular expression. The literal flavour escapes
the pattern, so `foo(` and `a.b` find themselves rather than being read as
syntax; there are no wildcards. Reach for `/` when you want them — including
when a search has to match case, which `(?-i)` asks for.

Runyte passes `/` queries directly to Rust's `regex` engine. The opening `/` is
the key that opens the prompt, not a delimiter around a JavaScript-style
`/pattern/flags` expression: use `(?i)hello`, not `/hello/i`, for a
case-insensitive regex. Inline flags include `i` (case-insensitive), `m`
(line-oriented `^` and `$`), `s` (`.` also matches newline), `R` (CRLF-aware
multiline boundaries), `U` (swap greedy and lazy repetition), `u` (Unicode,
enabled by default), and `x` (verbose mode). Flags can be scoped, as in
`(?i:hello)`. Character classes, alternation, groups, greedy and lazy
repetition, anchors, word boundaries, Unicode properties, and `\d`, `\s`, and
`\w` are supported. Slash-delimited expressions, trailing flags, look-around,
and backreferences are not. Capturing groups compile, but Runyte selects only
the complete match. Use `:help regex` for examples and a compact syntax table.

Buffer regex search runs over the complete buffer text, so `(?s)foo.*bar` or
an explicit `\n` can span lines. Workspace regex search remains line-scoped
because each result names one source row, so `Space / /` cannot return a
multiline match.

Every flavour selects *all* of its matches at once, so the edit that follows
applies to all of them. Each match is selected in full with the cursor on its
last character, which is where an append or a motion continues from. The whole
primary match has a light-orange selection and an orange cursor, while the
other matches stay blue without cursor blocks. The status line identifies the
primary's position among the results. Pressing `n` or `N` selects only the next
or previous match; later presses keep cycling that single selection through the
remembered results. An immediate edit therefore changes every match, while an
edit after search navigation changes only the match you reached. A selection
motion such as `e` turns the results into ordinary selections, restores their
orange endpoint cursors, and replaces the search status with the current
selection count.

A search runs over the whole buffer unless something is selected. When at least
two characters are selected — from `v` and a motion, from `x`, from `%`, or from
a previous search — the search looks only inside that text, and `n` and `N` wrap
within it rather than escaping into the rest of the file. Successive searches
therefore narrow: select a few lines with `x`, find the calls in them with `s`,
then pick out one argument with `/`. Press `;` to collapse back to a caret when
you want the whole buffer again.

Searching this buffer is two bare keys and no namespace, because two keys are
already the short spelling. `Space /` widens exactly those letters to the whole
project: the sigil says search, the prefix says the project rather than the file
in front of you, and the letter after it is the one the bare key already uses.
`Space / /` repeats the namespace letter for the flavour reached for most, the
way `Space b b` and `Space m m` do. Neither flavour has a case-sensitive
counterpart; write `(?-i)` in a regular expression when a search has to match
case.

`Space s` is selections only, and holds nothing that looks past them.

| Key | Action |
| --- | --- |
| `s` | Search the buffer, ignoring case |
| `/` | Search the buffer with a regular expression |
| `n` / `N` | Select only the next / previous match |
| `*` | Select every occurrence of the word or selection under the caret |
| `f` | Find the next typed character in the buffer |
| `Space / s` | Search the workspace, ignoring case |
| `Space / /` | Search the workspace with a regular expression |
| `Space / g` | Fuzzy-search file contents below the project root |
| `Space / f` / `Space f` | Find project files, open buffers, or terminals; `Tab` switches modes and `Ctrl-t` toggles preview |
| `Space s c` | Keep only the primary selection in any multi-selection |
| `Space s e` / `Space s b` | Put a cursor at the end / start of every selected line |
| `Space s a` or `&` | Pad with spaces until every cursor shares the rightmost display column |
| `Space s k` / `Space s r` | Keep / remove selections matching a typed regular expression |

The directory-scoped pickers have no key. `:file-picker-directory` and
`:fuzzy-grep-directory` search below the active file or explorer directory.

The two fixed workspace searches replace and open one read-only
`[workspace search]` buffer. Its `path:line:column` rows are a query-time
snapshot with typed destinations: Enter opens the result under the cursor,
and normal movement, selection, copying, `/`, `s`, splits, help, buffer
switching, and jump history keep their ordinary meanings. Visit as many
results as needed without rerunning the query; rerun a workspace search to
refresh and replace the singleton result view. This is deliberately different
from `Space / g`, whose fuzzy query remains live and therefore stays a
choose-one picker.

### Layout and whitespace display

`Space p` groups text presentation and the commands that lay selected lines
out. `Space p .` toggles visible whitespace for the current session.
Spaces become `·`, tabs begin with `→` and retain enough following cells to
reach the same tab stop, and a real LF or CRLF line ending becomes one `↵`.
An unterminated final line has no marker. These symbols are display-only: they
do not change buffer text, offsets, selections, wrapping, or saved files.

`Space p w`
hard-wraps every selection at `editor.hard_wrap_width` (80 by default).
Existing newlines remain boundaries, and words are kept intact unless one word
alone is wider than the configured line width.

`Space p r` uses the configured hard-wrap width and refills selected prose
instead of preserving every existing newline. Blank lines remain paragraph
boundaries. In Markdown files it keeps headings, fenced and indented code,
block quotes, tables, thematic breaks, and separate bullet or numbered list
items; wrapped list continuations receive a hanging indent. In recognized
source files it changes only `#` and `//` line-comment paragraphs, repeating
the indentation and comment leader on every output line, so selecting nearby
code cannot join code statements together. Lists inside line comments receive
the same hanging indent treatment. A word wider than the available content
width stays intact.

`Space p s` toggles `editor.soft_wrap` for the current session. Soft wrapping
always follows the live pane width, including after a resize; it does not use
the hard-wrap width and does not change buffer text. It uses the same
word-boundary rule, falling back to character boundaries only for a word wider
than the pane.

A document whose longest line exceeds 64,000,000 bytes is shown unwrapped
whatever `editor.soft_wrap` says. The length is measured once, when the file is
read, so the decision is made before anything is drawn.

The limit is set from measurement. Wrapping is computed per logical line and
from that line's start, so a frame's wrapping cost is linear in the longest
line: roughly 10ms for a one-million-character line, 160ms at sixteen million,
and 750ms at sixty-four million. The limit sits where a frame approaches a
whole second, which is where a document stops being slow to scroll and starts
being impossible to use. Below it nothing is taken away — a minified file of a
few megabytes still wraps, at about 17ms a frame — so the limit only refuses
the cases where wrapping could not have worked at all.

`Space p j` is the inverse of `Space p w`: it removes every line break inside
the selection. Select the lines first, with `x` or `X` for whole lines or with
`v` for part of them. It then asks what to put where each break was and joins on
Enter, so `Space p j Enter` runs the lines together, `Space p j Space Enter`
separates them with a space, and any other text — `, `, ` | `, a word — is
inserted literally. The whitespace sitting against each removed break goes with
it, so joining indented lines does not leave a run of spaces behind; the first
line keeps its own indentation. Only what is selected is joined: a selection
covering a single line reports that it holds no line break rather than pulling
up the line below it, and the line after the selection is never drawn in, not
even when a pointer drag ends on it. A selected blank line is still a line, so
it joins as an empty piece and leaves a delimiter of its own behind. Every
selection is joined in one transaction, so multiple selections and a single
undo both behave as one edit.

`Space p t` aligns the columns of the selected table, padding every cell to the
widest one in its column, so

```
| Column 1 | Column 2 |
|---|---|
| Value | abc |
| Longer text | Very very long text |
```

becomes

```
| Column 1    | Column 2            |
|-------------|---------------------|
| Value       | abc                 |
| Longer text | Very very long text |
```

A table is rows opening with `|`, cells divided by `|`, and at least one
separator row of dashes among them. The separator is what makes it a table
rather than prose or a closure that happens to start with a pipe, so a
selection without one is refused even though every line in it looks like a row.
It does not have to be the second line: the separator is recognized wherever it
falls, so a selection may start on it or end below a footer rule.

How the table was drawn is kept: a separator written `+---+---+` comes back
with its `+` signs, and GitHub's `:---`, `:---:`, and `---:` alignment colons
survive and decide whether the column's content sits left, centred, or right.
An escaped `\|` stays inside its cell. A tab inside a cell is expanded to
spaces at `editor.tab_width` stops, because a tab is only as wide as its
distance to the next stop and a column boundary cannot be worked out from one.

Select the rows first; unlike the other `Space p` commands this one widens the
selection to whole rows, because a table row is only a row from its opening `|`
to its close. Selections that then land on the same rows, or on consecutive
ones, are formatted together as the single table they cover. Blank lines inside
the selection are allowed and left as they were, so selecting a little more than
the table is fine, and the rows below the selection are never drawn in. Rows
that disagree on how many cells they hold are squared up with empty ones rather
than rejected, and no cell is ever dropped. All rows take the indentation of the
first. If any line in the selection is neither blank nor a row, or no separator
is among them, nothing is edited and an ERROR notification reports that no table was
detected.

### Macros

Macros live in one namespace instead of on two lonely letters. `Space m m`
starts a recording and the same keys end it, so nothing has to be remembered
to finish one; the keys that spell the stop are not part of what was recorded.
A recording captures raw input in arrival order and replays it through the
same dispatch that produced it, so a replay does exactly what the typing did.

`Space m m` records into the default macro, which is the register named `@`.
`Space m M` asks for a register first, and `Space m R` replays one; both accept
any printable key as the name. Only one recording runs at a time: starting a
second one while the first is open is refused rather than silently replacing
it.

Replay advances between frames rather than holding the editor's input loop.
`Escape` or `Ctrl-c` cancels the work still queued; inputs that already ran are
kept because a macro may invoke actions that cannot be rolled back. Direct and
mutual recursion are refused with the register chain that caused the cycle.
Distinct acyclic macro calls may nest up to 16 levels; reaching that defensive
limit also stops the top-level replay.
One top-level replay has a 10,000-unit work budget shared by raw key events,
characters in literal text, counted command repetitions, and nested macros.
Counted commands are expanded between frames instead of running their whole
count at once. Grammar-level range operations whose exact semantics cannot be
split are limited to 128 repetitions in one recorded input and are refused
before taking effect when larger. Reaching either safety limit stops the whole
replay. While work remains, the interaction line shows its progress and the
cancellation keys.

| Key | Action |
| --- | --- |
| `Space m m` | Start recording the default macro, or stop the recording |
| `Space m M` then a register | Start recording a macro under that register |
| `Space m r` | Replay the default macro; accepts a count |
| `Space m R` then a register | Replay that macro; accepts a count |
| `Space m l` | List every recorded macro; Enter replays the selected one |

### Files and splits

| Key | Action |
| --- | --- |
| `Space c y` / `Space c p` / `Space c P` | System clipboard yank / paste after / paste before |
| `Space e` | Open the active buffer's directory as an editable explorer; from a file, select that file |
| `Space E` | Open the working directory (controlled by `:cd`) as an editable explorer |
| `Space Space` | Open the persistent-session manager (`:session-list`, `:sl`); another `Space` closes it, `1`-`9` attach to a numbered session while the filter is empty, and `Tab` shows the selected row's actions |
| `Space / f` / `Space f` | Find project files, open buffers, or terminals; `Tab` switches modes and `Ctrl-t` toggles preview |
| `Space / g` | Fuzzy-search contents below the stable project root |
| `:file-picker-directory` | Fuzzy-find a file or directory below the active file/explorer directory |
| `:fuzzy-grep-directory` | Fuzzy-search contents below the active file/explorer directory |
| `Space b b` | Open the filterable buffer picker; `Ctrl-t` toggles preview and `Tab` shows valid actions |
| `Space b c` | Close the active buffer safely (`:close`, `:c`) without changing the pane layout |
| `Space b d` | Compare a fresh immutable disk revision with the active file buffer (`:diff-disk`) |
| `Space b n` | Open a new scratch buffer in the current pane (`:buffer-new`, `:new`) |
| `Space / /` | Search the workspace with a regular expression; see [Search](#search) |
| `Space r` | Reload the active text file or refresh the active explorer or supported Git list |
| `Enter` in a directory | Open the selected file or directory |
| `-` or Backspace in a directory | Open the parent directory and select the child just left |
| `.` in a directory | Show or hide dotfiles in the explorer |
| `x y` / `x d`, then `p` in a directory | Copy / cut selected entries, then paste in the destination explorer |
| `:w` or `:write` in a directory | Review a filesystem plan before applying edits |
| `Space w v/s` or compatibility `Ctrl-w v/s` | Vertical/horizontal splits |
| `Space w…` or compatibility `Ctrl-w…` | Window focus, close, next, and only-window operations |
| `Space w =` | Equalize pane widths, then pane heights within each column |
| `Space w f` / `Space w z`, or compatibility `Ctrl-w f` / `Ctrl-w z` | Toggle the full-screen pane / the centred Zen viewport |
| `Ctrl-h/j/k/l` | Move between panes without a prefix, when `editor.fast_pane_keys` is on |
| `Space o o` | Open the typed settings menu |
| `Space o t` | Preview and save a theme in `config.yaml`; `Tab` narrows to the dark or light ones |
| `Space o s` | Inspect syntax and LSP service health |
| `Ctrl-s`, `:write`, or `:save` | Save |
| `:diff-disk` | Compare the active file buffer with a fresh immutable disk snapshot |
| `:diff-this` (`:difft`, `:dt`) | Mark this buffer, or compare it with the one marked before it |
| `:diff-off` (`:do`) | Close the comparison this buffer is part of |
| `:reload` | Reload the active file (confirming first when dirty), or refresh the active explorer or supported Git list |
| `:path` | Show the active buffer's absolute path in a wrapped popup; `Tab` opens a menu to copy it to the system clipboard (`s`) or the unnamed Runyte register (`r`) |
| `:resize-right +/- N` | Grow or shrink the active pane at its right edge by `N` terminal cells |
| `:resize-left +/- N` | Grow or shrink the active pane at its left edge by `N` terminal cells |
| `:resize-top +/- N` | Grow or shrink the active pane at its top edge by `N` terminal cells |
| `:resize-bottom +/- N` | Grow or shrink the active pane at its bottom edge by `N` terminal cells |
| `:close[!]` or `:c[!]` | Close the active buffer in place; `!` explicitly discards unsaved text; terminals are refused |
| `:window-close` or `:wc` | Close the active pane, but refuse the last pane |
| `:quit[!]` or `:q[!]` | Close the active pane and its uniquely displayed buffer; from the last pane, exit standalone or stop the persistent session with unsaved-change protection |
| `:quit-all[!]` or `:qa[!]` | Exit standalone or stop the persistent session regardless of pane count, with unsaved-change protection; never terminate terminals |
| `:quit-here[!]` or `:qh[!]` | Quit and let the shell wrapper change to the active explorer/file directory |

Panes are reached from `Space w` or its `Ctrl-w` compatibility alias, both of
which want two keystrokes before a direction. Setting `editor.fast_pane_keys`
to `true` binds the bare `Ctrl-h/j/k/l` to the same four moves, the way a tmux
user navigates panes, in NORMAL, SELECT, INSERT, and a live terminal alike.
It is off by default because those four keys are not free: turning them on
takes `Ctrl-j` (insert newline) and `Ctrl-k` (delete to end of line) away from
Insert-mode editing, and takes all four away from a terminal's child, where
`Ctrl-h` is backspace and `Ctrl-l` clears the screen. Both prefixed spellings
keep working, and help and the key-hint popup describe whichever set is
actually in force.

`Space w =` levels the splits again after the `:resize-*` commands or a
dragged border have skewed them. It gives every pane the same width and then
gives every pane sharing a column the same height, moving only the boundaries
between panes: which pane sits beside or above which is exactly what it was. A
pane that spans the full width of the editor above a row of others therefore
keeps spanning it, and the row below it is what gets levelled.

Closing an explorer retires its buffer and keeps every pane. Each pane that was
showing it returns to its own most recently used live buffer, then any live
buffer, or a scratch buffer when none remains. `:c` refuses unapplied explorer
edits; the typed-only `:c!` drops the plan without touching the filesystem.

Service health is an informational report overlay: arrows and paging scroll,
Escape dismisses it, and printable input or Enter cannot accidentally turn it
into a filtered choice. Filesystem-plan confirmations likewise scroll with
arrows, paging, Home, and End and show the reviewed operation and total before
Enter applies with trash deletion or the separately advertised `P` applies
with permanent deletion.

The editable explorer is a normal directory buffer: use the editor's movement
and editing commands, Enter to open an entry, `-` or Backspace for the
parent, `r` to refresh, and `.` to show or hide dotfiles. Enter is the only
key that opens an entry, so `e` stays the word-end motion it is in every other
buffer. Renames, additions,
moves, copies, and removed lines reach the filesystem only after `:w` or
`:write` presents a plan and Enter confirms it. Select entries with `x`, use
`y` to copy or `d` to cut, navigate or focus the destination explorer, and use
`p` to paste. The register retains the filesystem identity across navigation
and panes, so pasted entries keep their contents and directories remain
recursive operations. A directory cannot be moved below itself: editing
`dir/` into `dir/child` is rejected by `:w` before any confirmation opens.
Trailing whitespace and whitespace-only rows do not represent filesystem
changes; writing a listing with only those edits refreshes
it to the canonical directory view. A filename included in the listing that
ends in whitespace, contains a control character, or is not valid UTF-8 makes
the editable projection refuse to open, because such a name cannot be
distinguished safely from the explorer's row syntax. New rows are held to the
same boundary before a confirmation opens.

Dotfiles start out listed or not according to `editor.show_hidden_files`, and
`.` flips that for the session without writing it to `config.yaml`. Because
the preference is one value, `.` re-reads every clean explorer rather than
only the active pane. A listing without its dotfiles is also planned without
them: an entry it never showed is neither deleted for being absent from the
text nor reported as a change when it appears, and a name colliding with one
still stops the plan instead of overwriting it. An explorer holding unsaved
edits refuses the toggle, since re-reading would discard text that has not
been through a write plan.

A symlink is listed under its own name, followed by a muted `→ target` hint
showing what it points at, exactly as the link stores it. The hint is not part
of the buffer: it cannot be selected, edited, or written, and every hint in one
listing starts in the same column. Enter on a symlink opens what it points at,
so the file you edit is the one Git and the language server know about rather
than the link, and a broken link says so instead of opening. Renaming,
copying, cutting, and deleting still operate on the link itself.

Each pane browses with one explorer, retargeted as it walks: however deep you
go, `Space b b` lists a single directory buffer per pane rather than one per
directory visited. Its picker row is labelled `[explorer] dirname`, followed
by its project-relative path (typically `.` for the project root). A directory normally comes back with the row you left it
on; moving to its parent with `-` or Backspace instead selects the child just
left, so Enter immediately returns to it. If that child is hidden by the
explorer's dotfile filter, the parent's remembered row remains selected.
A split gets an explorer of its own, so two panes can still show two different
directories. Because retargeting throws the listing away, refreshing or
navigating away from an explorer with unsaved edits asks before discarding
them.

Splitting an explorer works exactly as splitting a file does: the new pane
shows the same listing, on the same row, and no key opens the entry under the
caret in a split. What the new pane becomes is then your choice — navigate it
to the destination directory and it takes an explorer of its own, which is
what makes a copy or a cut across two explorers possible.

In the buffer picker, Enter opens the selected buffer, `Ctrl-t` toggles its
bounded preview of authoritative in-memory text, and Tab opens contextual
actions. A modified file offers Save and Discard changes; discard
requires a separate Enter confirmation. Close appears only after a buffer is
clean, never discards edits implicitly, and redirects every pane that shared
the closed buffer. The first column shows a file or directory name, or the
existing structural name of another buffer type; the active name is surrounded
by `*`, and read-only types carry `[RO]`. The second column is reserved for file
and directory paths, relative to the project root when they are inside it and
absolute otherwise. Explorer buffers deliberately expose no management
actions in this view: filesystem changes remain available only by editing the
explorer and confirming its `:write` plan.

Runyte retains the two most recently active clean generated and special
buffers, including explorers, after their last pane moves elsewhere. Activating
a third retires the least recently used detached one, which keeps immediate
`Ctrl-o`/`Ctrl-i` and `Alt-o`/`Alt-i` navigation useful without letting
`Space b b` or the file picker's buffer mode accumulate stale views. A dirty
special buffer remains open and discoverable until it is saved or explicitly
discarded. An empty, clean scratch buffer still retires as soon as its final
pane leaves; written scratch text remains an ordinary buffer.

After an applied plan, Runyte refreshes other affected clean explorers and
retargets open files through the confirmed rename/move identities. Dirty
affected explorers are preserved with a warning, and deleted open paths are
reported as stale instead of being silently redirected.
The explorer that applied the plan keeps its edited row order, so a renamed or
new entry stays where it was written. Entering that clean explorer again after
visiting a file or another directory restores the canonical sorted listing.

Preparing a plan records the complete trees of directories it will move, copy,
or delete. A change anywhere below one of those directories before confirmation
is applied rejects the whole plan before any operation begins.

The project finder opens in file mode. `Tab` switches between project files
and a combined view of open buffers plus terminals without clearing the query.
Both modes preview the selected item, with authoritative in-memory text for
buffers and bounded recent output for terminals; `Ctrl-t` toggles the preview
without changing the mode or query. The existing
`Space b b` and `Space t t` managers remain available for Save, Discard,
Close, Rename, and other contextual actions; the combined finder is an
Enter-to-open surface.

The buffer-and-terminal mode matches each space-separated query term against
any indexed field. Buffers contribute their structural type and their file or
directory path. Terminals contribute their assigned name, child title, launch
program, stable ID, current reported directory, and initial directory. Paths
are searchable as absolute, project-relative, `~/`-relative, or basename-only
spellings. `terminal`, `term`, and `buffer` are soft type hints: they move that
kind first without hiding other candidates that match the remaining terms.

The native fuzzy file picker recursively discovers regular files and
directories without invoking `git`, `find`, `fd`, or an external fuzzy finder.
It honors nested `.gitignore` and `.ignore` rules, including ancestor rules
when `:file-picker-directory` starts below the project root. It never follows symlinks,
refuses to scan `.git`, `.runyte`, or the configured workspace state
directory, and applies `editor.show_hidden_files`.
Type an ordered subsequence to rank paths; exact basenames, basename prefixes,
consecutive characters, and path-component boundaries rank highest. Ending the
query with `/` narrows the results to directories, matched without the slash
itself. Matching characters are highlighted and the selection is previewed:
a text file from its first 64 KiB, even when the file itself is large, and a
directory as a one-level listing of its files and subdirectories; `Ctrl-t`
toggles that preview. Enter opens a file for editing and a directory in the
editable explorer.

Use Up/Down, `Ctrl-p`/`Ctrl-n`, paging, and Home/End to move. In the project
finder `Tab` switches modes; `Shift-Tab` selects the previous row, as it does
in directory-scoped and fuzzy-content pickers. In those other pickers, `Tab`
retains its previous next-row navigation.
Enter opens, `Ctrl-s` opens horizontally, `Ctrl-v` opens vertically, and
Escape or `Ctrl-c` closes. A bare `Space` also closes a newly opened overlay;
after a project or content finder query has begun, it retains its term-separator
role. Backspace/Delete and the ordinary prompt control keys edit the query.
Printable letters such as `q`, `j`, and `k` are query text rather than
navigation commands.

A space separates the query into terms rather than being matched. One word is
the fuzzy subsequence it has always been, so `fpick` finds `file_picker.rs`.
Two or more words each have to be present as themselves, in the order they were
typed: `src picker` finds `src/picker.rs` without the incidental matches a
subsequence through `s…r…c…p…i…c…k…e…r` collects, and `content entries` finds
the eighteen lines holding both words rather than 272 lines holding their
letters in order. Because the terms are wanted in order, `picker src` does not
find `src/picker.rs`. Smart case reads the whole query: one capital anywhere
makes every term case-sensitive. Both pickers and fuzzy grep share this rule.

The fuzzy grep picker uses the same interaction to search file contents rather
than paths. `Space / g` scans from the project root and `:fuzzy-grep-directory`
scans from the active file's or explorer's directory. It streams non-empty UTF-8 lines
from ignore-aware files, ranks the typed query against the line text, and
displays `path:line` in the result list while the selected file's content
remains in the preview. The preview starts with nearby context, marks the
matching line with its real line number, and fills a direct match with the
primary match colour — one that landed whole, meaning a single word on a
contiguous span or every term of a several-word query on a span of its own. A
match with a gap inside a term fills its individually matched characters with
the secondary match colour.
Unsaved open buffers replace their stale disk contents, and Enter opens the
selected location. Files larger than 4 MiB are skipped.

The search runs in the scan rather than after it: editing the query restarts
the background walk, which keeps only the lines that query matches. The
50,000-candidate bound that keeps ranking interactive is therefore a bound on
matches, not on how much of a project was read, so a match is found wherever
in the project it lives. Typing on after a complete scan narrows what is
already on hand instead of walking again. `result limit reached` in the picker
title means the project holds more than 50,000 matches for the query, which a
longer query resolves — on a project of 150,000 lines only a single-character
query reaches it.

Opening a binary file — one whose bytes hold a NUL or do not decode as UTF-8 —
asks which program should have it instead of loading it as text. An initial
eight-kilobyte probe avoids reading an obvious binary twice; the final read
still validates the complete file before it becomes a buffer. Type a name
found on `PATH` or an absolute path; Enter hands the file
over and Esc leaves it alone. Recent choices are offered above the prompt, most
recent first, with `↑`/`↓` to select and Tab to complete, and the most recent
one seeds the prompt. The list is cached under Runyte's platform cache
directory (`$XDG_CACHE_HOME/runyte`, `~/.cache/runyte` on Linux when unset,
or `~/Library/Caches/runyte` on macOS), so it remains separate from project
state. The program is started detached with no terminal of
its own: viewers and GUI applications work, a terminal program cannot take
over the screen.
System clipboard commands use `pbcopy`/`pbpaste` on macOS, PowerShell on
Windows, and the first available of `wl-clipboard`, `xclip`, or `xsel` on
Linux and other Unix systems. A missing helper produces an actionable status
message without affecting the internal registers.

### Terminals

| Key | Action |
| --- | --- |
| `Space t n` | Run `$SHELL` in this pane (`:terminal`, `:term`, `:t`; `:terminal <command>` runs something else) |
| `Space t t` | List the running terminals and show the chosen one here (`:terminals`) |
| `Space t r` | Rename this pane's terminal (`:terminal-rename <name>`) |
| `Space t q` | Show this pane's buffer again, leaving the program running |
| `Space t y` | Copy this terminal's output into a read-only buffer (`:terminal-output`) |
| `Space t s` | Send the selection — or the whole buffer — to a terminal as one bracketed paste (`:terminal-send [id\|name]`) |
| `Tab`, then Close in `Space t t` | Explicitly end and forget the selected terminal |
| `Ctrl-w h/j/k/l` or arrows in Terminal Insert | Move directly without capturing or discarding review; a live terminal destination starts Insert, a reviewed terminal stays in review, and a document destination starts Normal |
| `Ctrl-h/j/k/l` in Terminal Insert | The exact same destination behavior without the prefix, when `editor.fast_pane_keys` is on; the child stops receiving those four keys |
| `Ctrl-w w` in Terminal Insert | Cycle panes with the same live-terminal/reviewed-terminal/document destination behavior |
| `Ctrl-w v/s` in Terminal Insert | Create a vertical/horizontal document split while leaving the terminal child live without review |
| `Ctrl-\` in a terminal | First leave INSERT for live NORMAL, then enter review on the second press. Reported as `Ctrl-4` by terminals without the enhanced keyboard protocol, and both work |
| `i` / `a` in a terminal | Type again, returning to the live screen first |
| `h` / `j` / `k` / `l`, word, line, paragraph, and character-find motions | Move the terminal review caret; after `v`, extend an inclusive character selection in either direction |
| `%` in terminal review | Select all retained review text |
| `x` / `X` in terminal review | Select the current line, then extend the moving edge down / up on repeated presses |
| `C` / `Alt-C` in terminal review | Add carets below / above at the same occupied terminal-cell column, skipping short rows |
| `Ctrl-u` / `Ctrl-d`, `Ctrl-b` / `Ctrl-f` | Move the review caret by half / full pages, keeping it visible |
| `gg` / `ge` in a terminal | Move to the oldest / newest rows in the captured review snapshot |
| `gw` in terminal review | Label visible terminal words and jump to the chosen one |
| `s` / `/`, then `n` / `N` | Search an immutable terminal review snapshot and move among matches |
| `y` / `Space c y` in terminal review | Copy the caret character or every selection, joined by newlines, to the unnamed register / system clipboard |
| `p` / `P` in a terminal | Leave review and send Runyte's selected register to the live program |
| `Space c p` / `Space c P` in a terminal | Leave review and send the system clipboard to the live program |

In INSERT mode every key belongs to the program except `Ctrl-\`, the
`Ctrl-w` window prefix, and `Ctrl-h/j/k/l` while `editor.fast_pane_keys` is
on. NORMAL/review mode navigates and copies but
does not edit; commands that would edit are refused rather than applied to the
buffer waiting behind the pane. See [Terminals](#terminals) for what the
emulator supports and what it does not.


### Git

| Key | Action |
| --- | --- |
| `Space g b` | List local branches and check one out |
| `Space g g` | Open the changed-file list |
| `Space g w` | Open the repository worktree list |
| `Space g d` | Open the active file's unstaged diff |
| `Space g D` | Compare the active file's complete Git versions side by side |
| `Space g l` | Open paged commit history |
| `Space g /` | Fuzzy-search commits in a hash/title list with author, date, and full-message preview |
| `Space g B` | Open live-buffer attribution for the whole file |
| `Space g t` | Open the bounded stash list |
| `Space g r` | Re-read branch, changed files, and changed lines from Git |

`Space g b` opens a read-only local branch list. The current branch is marked
with `*`. Every branch currently checked out in a registered worktree carries
each checkout's local path in a `[worktree: ...]` annotation; move through the
list as through any buffer and press `Enter` to check out the branch under the
cursor. Runyte refuses the checkout while the index or working tree has
staged, unstaged, or untracked changes, and also
while an open file buffer in the repository has unsaved edits. After a
successful checkout, open files that still exist are reloaded from the new
branch and the Git status and gutter bases are refreshed.

```text
  feature [↑1] [worktree: /home/me/project-feature]
* main    [↑2 ↓1] [worktree: /home/me/project]
  spike
```

A branch that tracks a remote-tracking branch carries its drift in brackets,
lined up in a column of its own and dimmed apart from the name: `↑` counts
commits the local branch has that its upstream does not, `↓` counts the
reverse, `[=]` means the two are in step, and `[gone]` means the upstream ref
no longer exists. A branch tracking nothing says nothing.

| Key | Action in the list |
| --- | --- |
| `Enter` | Check out the branch on this line |
| `Tab n` | Start a new branch here and switch to it |
| `Tab D` | Delete this branch, with its worktree and session, after a confirmation |
| `Tab p` | Fast-forward the current branch onto what it tracks |
| `Tab P` | Publish this branch to what it tracks |

`Tab n` asks for a name, creates that branch at the one under the cursor, and
switches to it — the same two refusals as a checkout apply, and they are
reported before the name is asked for rather than after. `Tab D` first reviews
the exact branch tip. When its commits are retained by the configured upstream
or another local branch, Enter confirms; the confirmation names the retaining
local branches. When no such ref retains the tip, the exact branch name must be
typed before Enter accepts it. Upstream reachability uses the locally cached
remote-tracking ref and says so in the confirmation; fetch first when that
answer needs to include newer remote state. A branch checked out in a registered
worktree is not refused. A worktree means nothing without the branch it has
checked out, and a session means nothing without the worktree it was opened on,
so deleting the branch offers to take all three down at once. The confirmation
names every level and always requires the exact branch name, whatever the tip
alone would have settled for:

```text
Delete branch enh/render-space.
This also:
  · stops and forgets session 5 (runyte-enh-render-space)
  · removes worktree /home/me/code/runyte-enh-render-space
Type enh/render-space exactly to continue.
Escape keeps it.
```

Accepting runs bottom-up: the session stops, then the worktree is removed, then
the branch is deleted. A failure at any level stops the cascade there rather
than continuing into the one below it, and the checkout's own refusals — a
dirty working tree, a lock, a session holding unsaved buffers — are reported
before anything is put to you as a choice. A branch reported as checked out in
more than one worktree, or at the current Runyte root, is still refused before
review. The final mutation rechecks the tip and its
retaining refs, so a branch changed after review is not deleted.

`Space g w` (equivalently `:git-worktrees`) opens every checkout registered with the repository, including
linked, detached, locked, prunable, and bare worktrees. The current root is
marked with `*`; unavailable states are written on their rows. Paths are the
stable identity, including paths that are not valid UTF-8 even though their
display uses replacement characters.

| Key | Action in the worktree list |
| --- | --- |
| `Enter` | Attach to this root's persistent session, starting it if necessary |
| `Tab n` | Create another checkout of this row's branch; attach to it in persistent mode |
| `Tab N` | Name a new branch and create its checkout; attach to it in persistent mode |
| `Tab D` | Remove this worktree and its session after confirmation; keep its branch |
| `Space g r` | Re-read the registered worktrees |

Opening another root never retargets this workspace's buffers or language
servers. In persistent mode, Enter detaches the TUI and reuses or starts the
destination root's host, leaving the old host and its buffers and terminal
sessions alive. Successful `Tab n` and `Tab N` creation immediately performs
the same attachment, so a newly created worktree needs no separate Enter.
Standalone Runyte keeps the worktree list and its create/remove Git actions,
but Enter explains that attachment needs `workspace.mode: persistent` and
creation stays in the current workspace. `Tab D` removes one selected ordinary
worktree at a time and never deletes its branch. Staged, unstaged, or untracked
files refuse removal before confirmation, as does a running persistent session
with unsaved file buffers or unavailable health.

A *clean* session on that worktree does not refuse the removal — it goes with
it. The confirmation says so, and asks for typed text for that reason alone
even where the worktree's own state would have accepted Enter:

```text
Remove worktree /home/me/code/runyte-enh-render-space.
This also stops and forgets session 5 (runyte-enh-render-space).
Branch enh/render-space will remain.
Type enh/render-space exactly to continue.
Escape keeps it.
```

The session is stopped before Git is asked to remove the directory, because the
host owns that directory and the runtime state under it, and the removal itself
has to report success before anything below it happens: a removal Git refuses
after the confirmation leaves the branch and the history record alone. A stop
that fails likewise leaves the worktree standing. Once the directory is gone the
workspace's history record is forgotten too, which is what frees its number for
the next workspace; that happens whether or not a host was running, so a
worktree opened as a workspace at any point leaves nothing behind.

This part of removal is not a persistent-mode feature. A standalone editor
cannot attach to a session, but it can still find one running on the worktree it
is removing, and stopping that host is part of the removal rather than a
`session` command — so it happens in either mode.

A clean worktree whose branch
has commits ahead of its cached upstream (or whose upstream is gone) requires
the exact branch name; an unretained detached checkout requires its displayed
path. Other clean removals use Enter. Git status, worktree identity, upstream
state, and persistent-session health are checked again after confirmation. The
current Runyte root, locked, bare, missing, and otherwise unavailable worktrees
are refused before confirmation.

`Space g l` opens commit history in pages of up to 10,000 commits, newest first
in Git's topological order. The first line gives the current and total page
counts, its earliest and latest author dates, and a reminder of how to move
between pages, separated by `|` and all within 80 characters. The paging
reminder is a muted, read-only hint rather than buffer text, so it cannot be
selected, searched, or copied. Each row shows the short object ID, the author
date as `YYYY-MM-DD`, the author, and the subject, while keeping the full
object ID behind it. A commit's branch and tag refs, when it has any, are
shown the same way — a muted, read-only hint rather than text appended to
the subject. Unlike the explorer's symlink hints, a commit's ref hint is not
aligned to a shared column: it sits one space past that row's own text, so
one commit with an unusually long subject or long ref list never pushes a
shorter row's hint off a narrow pane.

Enter opens bounded commit metadata and Git's patch. `Ctrl-n` and `Ctrl-p` move
to the next and previous page — paging deliberately sits on Ctrl chords so `l`
and every other motion keeps working in this view — and `Space g r` re-reads
the view. Pages use boundary objects instead of holding an unbounded result in
memory, so stepping back re-requests the cursor that produced the earlier page.

Refresh keeps the caret on the same commit whenever that object is still on the
page, even if a new commit appeared above it, and otherwise falls back to the
nearest row. Only the first page is refreshed automatically: every later page
sits behind a commit boundary, so its history cannot change underneath you.

`Space g /` opens a native fuzzy picker over commits reachable from `HEAD`,
newest first. Matching covers everything a row stands for rather than only what
its author wrote: subject lines, message bodies, the object ID, the author, and
the author date. A prefix of an object ID matches, so the abbreviated ID a row
shows is enough to find it. Rows show the subject, author date, author, and
abbreviated object ID. Enter
opens the same bounded commit detail used by the log and blame views. Discovery
is asynchronous and capped at the newest 5,000 commits; the picker says when
that limit was reached.

`:git-blame` attributes the primary line without leaving the file. `Space g B`
opens a read-only, source-row-aligned blame view; Enter there opens the row's
commit. Both send the current in-memory buffer text through Git's porcelain
blame input, so an unsaved line says `uncommitted` instead of borrowing an
older on-disk attribution. A result is discarded if the buffer changes while
Git is working. Inputs over 4 MiB and whole-file views over 20,000 lines are
refused with a limit rather than loaded without bound; untracked or otherwise
unblamable files report Git's refusal. Each row in the full-file view also
shows the commit's author date as `YYYY-MM-DD`, in the commit's own
timezone.

`Space g t` opens a bounded, read-only stash list whose rows retain full stash
object identities. `Tab a` applies the selected stash while retaining it,
`Tab D` drops it only after confirmation, and `Space g r` refreshes without
losing the selected object. Stash creation is deliberately split into commands
that name their
scope: `:git-stash-tracked <name>` records the tracked worktree and index
snapshots while leaving staged changes applied, `:git-stash-all <name>` records
the same tracked state and clears both worktree and index, and
`:git-stash-untracked <name>` additionally includes untracked files. Every
create, apply, and drop asks for confirmation, and create/apply refuse while
the repository has unsaved editor buffers. An apply conflict retains the stash
and reports that resolution belongs in an external Git tool.

`Space g g` opens the changed-file list: every file grouped by whether a commit
would take it, one file per line. Rows are files, so a selection over several
rows is a selection of files and one key acts on all of them.

```text
# main ↑1 · 2 staged · 1 not staged · 1 untracked · +105 -28

Staged
  M  +82  -12  src/app.rs
  A  +20   -0  src/git/stats.rs

Not staged
  M   +3  -16  README.md

Untracked
  ?    ·    ·  logo.png
```

The first row names the branch, how far it has drifted from its upstream, and
how many files are in each section below. It counts sections rather than
per-file states, so it never calls a file "modified" directly above a heading
that calls it staged — the status line's compact `~1` form does the latter,
because a glance at what changed is a different question from a breakdown of
where it sits.

Each text-file row carries what its change costs in lines, added then removed,
in one column aligned across the whole list, and the first row totals them. The
two counts are drawn in the theme's `change_added` and `change_removed` — the
same colours as the Git gutter and as added and removed lines in a diff — so a
theme decides what they look like and nothing here is red and green by fiat.
The numbers are Git's own `--numstat` counts of the same two trees the row
itself comes from, so a file that is staged and then edited again is counted
once on each of its rows and the total is the sum of what is shown. An
untracked file has nothing to be compared against, so every line in it counts
as added. When at least one row can be counted, a change whose lines cannot be
counted — a binary file, an untracked symlink, a file over a megabyte, or a
whole untracked directory Git collapsed into one row — shows `·` in both
columns rather than a zero, and is left out of the total. A list with no
countable rows omits the columns. The counts are read only while the list is
open.

| Key | Action in the list |
| --- | --- |
| `Tab s` / `Tab u` | Stage / unstage every file the selection covers |
| `Tab S` | Stage every unstaged and untracked file in the list |
| `Enter` | Show this row's diff |
| `Tab o` | Open the file on this line |
| `Tab D` | Discard the selected files' changes |
| `Tab c` | Write a message and commit what is staged |
| `Tab i` | Review everything staged for the next commit |
| `Tab p` / `Tab P` | Pull / push the branch this working tree is on |
| `Space g r` | Re-read everything from Git |

`Enter` follows the row it is pressed on: on a staged row it shows what a
commit would take, on an unstaged row what it would not. A file that is staged
and then edited again has a row in both sections, which is not a duplicate:
those are two different changes, staged separately. A selection covering both
still acts on the file once. After staging, the caret
follows the file into its new section, so `Tab u` undoes what you just
did rather than acting on whichever file closed the gap.

`Space g d` and `Tab i` open read-only buffers holding Git's own patch
text, coloured by what each line is: added and removed lines in the gutter's
own colours, hunk positions in the accent colour, and headings muted. The same
colouring applies to generated patch views. In a per-file diff, `Tab s` stages
the exact hunk under the cursor and `Tab u` unstages the exact staged hunk.
The request carries the hunk bytes plus repository, HEAD, index, file, and
live-buffer preconditions; Git checks the patch, and any stale precondition
changes nothing.

`:git-stage-lines` stages a deliberately narrow safe slice from
a saved, clean source buffer: one contiguous selection containing every added
or modified new line in one hunk. Dirty buffers, deletion-only choices,
multiple or partial hunks, binary files, conflicts, renames, and untracked
files are refused. Runyte never turns a refused partial action into whole-file
staging. Use Lazygit for finer patch surgery, conflict resolution, or other
advanced history work.

`Tab s` records each selected file **as written on disk**; when a buffer has
unsaved changes an ERROR notification says so rather than leaving you to wonder which
text went in. Staging moves the base the gutter is measured against, so the
marks for the lines you staged disappear as soon as they are recorded.
For a rename, the displayed destination remains the file opened or diffed,
while staging and unstaging act on both the original and destination paths so
the move cannot be split across the index boundary.

`Tab c` opens a commit message buffer holding the template Git would hand
an external editor: an empty first line, then commented instructions and the
files that will be recorded. Runyte refreshes the index before deciding whether
there is anything to commit, so staging done outside the editor is included.
Write the buffer with `:w` or `:wq` to commit;
`:wq` does not exit Runyte from this special buffer. Use `:c` for an unchanged
message or `:c!` for an edited one to cancel — nothing is committed and the
index is left exactly as it was. Comment lines are not part of the message, which is also why a
message line cannot begin with `#`.

Committing takes **the index** — exactly what the Staged section shows — so
there is one meaning of the word wherever it is invoked from. A refused commit
keeps the message in the buffer, so an unset identity or a hook that rejected
it is something to fix and retry rather than something that loses your work.
Afterwards the pane returns to where the detour started.

`Tab D` throws the selected files' uncommitted changes away, restoring them to `HEAD` —
both what was staged and what was not. It is the only Git action here that
cannot be undone: the discarded content was never a commit, so no reflog will
produce it again. It therefore asks first and names what it will take. A
selected file with unwritten buffer edits is refused so Git never overwrites
text that exists only in the editor; successful discards reload clean buffers
afterwards. Discarding a staged addition removes its clean open buffer along
with the file; discarding a rename restores the original path and removes the
destination.

Untracked files are refused rather than deleted. Discarding one could only mean
removing it, which Git keeps behind `clean` and Runyte keeps in the explorer,
where deletion is a confirmed plan that goes to the trash.

Hooks run inside the background commit operation. A slow `pre-commit` leaves
the editor responsive, and a signing key that wants a terminal prompt fails
rather than asking.

Pull and push are the two commands here that reach the network, and they live
in the `Tab` action menus of the branch list and changed-file list as `p` and
`P` mnemonics.

Git discovery, reads, mutations, hooks, pull, and push run on the bounded Git
service, so editing and rendering continue while they are queued or running.
Long-running mutations temporarily replace the normal status row with the
action, its target, elapsed time, cancellation hint, and a rotating `- \ | /`
bar at the right edge. This is a general TUI progress surface rather than part
of a Git buffer, so every background service using it gets the same spinner.
Use `:git-cancel` to stop the current Git operation; a cancelled mutation is
reported as uncertain and immediately reconciled because cancellation is not
rollback.
Network operations retain their two-minute deadline. Nothing can prompt while they run —
Git's own prompts are off, `ssh` is put in batch mode unless you have set
`GIT_SSH_COMMAND` yourself, and no askpass helper falls back to the terminal —
so an authentication that needs a password fails with a message instead of
hanging behind a prompt you cannot see. Repository- and object-selection
variables such as `GIT_DIR`, `GIT_INDEX_FILE`, and inherited one-shot Git
configuration are removed from every child environment, so starting Runyte
from another Git command cannot silently retarget its repository operations.
File arguments are always literal as well: a filename that resembles Git's
pathspec syntax still names only that file.

`Tab p` fast-forwards where it can, and asks where it cannot. A fast-forward is
silent, because it decides nothing: the branch had no commits of its own to
lose. When you and someone else have both committed to the same branch there is
no fast-forward, so `Tab p` says how far apart the two have drifted and offers
to replay your commits on top of theirs — `main and origin/main have both moved
on. Press Enter to replay 2 local commits on top of the 1 on origin/main`.
Escape leaves the branch exactly as it was.

The replay is a rebase, so it never leaves a merge commit whose message nothing
here could write, and if it hits a conflict it undoes itself: the rebase is
aborted, the working tree keeps what it held, and the refusal says so. Runyte
has no surface for resolving a conflict, so it never leaves you holding one —
finish those with whatever Git tool you already use. A rebase does rewrite the
commits it replays, which stay reachable from the reflog under their old
identities; that is why it asks rather than doing it silently.

`Tab p` refuses first while an open file buffer in the repository has unsaved
edits, since a pull rewrites files that are then reloaded. Afterwards, open
files are reloaded and the gutter bases refreshed, exactly as after a checkout.

Neither the pull nor the replay stashes uncommitted changes, even where
`merge.autoStash` or `rebase.autoStash` is configured. Git reapplies such a
stash after the work is done, and when reapplying it conflicts it says so and
still exits successfully — leaving conflict markers in the working tree and a
stash to recover, with nothing left to roll back. A dirty worktree is refused
up front instead, which is an outcome you can act on.

The fetch and the merge are also run as separate commands rather than as one
`git pull`, so an unreachable remote is reported as an unreachable remote. The
drift `Tab p` offers to replay is read from the remote-tracking refs, and those
are only worth reading once a fetch has actually refreshed them; a single failed
pull would not say whether its fetch got that far.

`Tab P` publishes to the ref the branch tracks, and sets an upstream the first
time — `origin` when it exists, or the only remote when there is exactly one.
Nothing forces: a push the remote rejects because it holds commits you do not
is reported as the refusal it is, and names `Tab p` as the way to catch up.
Fetch has no binding; `:git-refresh` re-reads what is already local.

### Language features

| Key | Action |
| --- | --- |
| `Space l h` | Show documentation for the symbol under the cursor |
| `Space l s` / `Space l S` | Document / workspace symbols |
| `Space l d` | Diagnostics |
| `Space l r` / `Space l a` | Rename symbol / apply a code action |
| `Space l c` | Ask for completions (`Ctrl-x` in Insert mode) |
| `Space l f` / `Space l R` / `Space l ?` | Format / restart language servers / report language-server state |
| `Space l g d/D/y/r/i` | Definition / declaration / type definition / references / implementation |
| `gd` / `gD` | Go to definition / declaration |
| `gy` / `gi` | Go to type definition / implementation |
| `gr` | Go to references |
| `:format` | Format the buffer (typed equivalent of `Space l f`) |

Hover documentation stays anchored to its source. A short document is a peek
that dismisses and redispatches the next key. When more than twelve lines are
available, the title states how much was omitted and Enter opens the complete
text in a retained read-only `[documentation]` special buffer.

A goto with one result moves the selection to it; several open a result
picker. Every picker filters as you type, moves with the arrows or
`Ctrl-n`/`Ctrl-p`, opens with Enter, and closes with Escape. In Insert mode
the completion popup appears after one of the server's trigger characters, on
Insert `Ctrl-x`, or when `Space l c` enters Insert mode and asks explicitly.
It then filters locally as you keep typing; `Tab` accepts and `Escape`
dismisses. In an editable explorer, `Escape` also leaves Insert mode in that
same press, because a directory row's structural `/` can open path completion
without an explicit request. An automatic word-completion popup behaves the
same way: `Escape` dismisses it and returns to Normal mode in one press.
An explicit `Ctrl-x` request includes the identifier already before the caret,
uses the server's `filterText` and `sortText` when present, and remains the
active completion source until space, newline, acceptance, or dismissal.
Typing punctuation does not hand that session to word or path completion; a
trigger character refreshes its candidates for the new language context. If the
current prefix has no matches the popup disappears without ending the session,
so Backspace can reveal matching LSP candidates again. Backspace and Delete
keep the session; moving the caret or running another editing command ends it.
Enter is deliberately not an accept key for any completion source: a popup
can open on its own — after a trigger character, or for any three-character
word prefix — so Enter always inserts its usual newline and dismisses
whatever was showing, rather than risking an unwanted candidate on a keystroke
meant to end a line. A completion that needs an import applies both edits as a
single undo step, and so does a rename across a file. All edits made between
entering Insert mode and returning to Normal mode are likewise one undo step;
undo and redo map the caret through the inverse edit instead of leaving it at
a stale character offset.

Path completion does not require a language server. In Insert mode, it opens
a filterable popup of a directory's files and subdirectories as soon as the
text immediately before the caret is a valid absolute or relative directory
followed by a fragment of a name, which reopens it on any keystroke that
leaves such a path before the caret — not only on typing `/`, so editing an
already-typed path (moving the caret and continuing to type) shows hints just
as typing it fresh would. Relative paths are resolved against both the active
file's parent and the stable project root, so spellings such as `dir/`,
`files/`, `./files/`, and `../files/` can follow whichever context makes them
valid. Directory candidates retain a trailing `/`; accepting one immediately
offers its children. A directory with more names than the popup will show is
offered as its first few hundred in order, and typing more of a name narrows
against the whole directory rather than against what the popup happened to be
showing, so a name that exists is reachable however large the directory it
sits in. The command palette's rows for a path argument work the same way.

Word completion offers words already seen elsewhere in the workspace — every
open buffer contributes, including the explorer, whose entries are filenames.
It needs no language server and no trigger key: once a typed prefix reaches
`editor.word_completion_minimum` characters (3 by default), the popup appears
on its own in file buffers, the scratch buffer, and the commit message, using
the same filtering, navigation, and acceptance as language and path
completion. A word contains Unicode letters and numbers. A hyphen stays part
of it only when it joins characters on both sides, so `up-to-date` stays whole;
all other punctuation is a boundary and is never included in a candidate.
This keeps the list focused on prose words, numbers, and source-code name
fragments. Candidates from the buffer being typed in come first, ordered by
how often each occurs there,
followed by words from every other buffer in the same order; the word
currently being typed is never offered as a completion of itself. A background
index maintains this off the main thread, so a candidate can be one keystroke
stale but a keystroke never waits on it. `Ctrl-x` still wins outright, replacing
a word popup as soon as the request is sent, and a path in progress takes over
the moment `/` is typed unless an explicit LSP session is active; word
completion never overrides either one. Turn
it off, or change the trigger length, with `editor.word_completion` and
`editor.word_completion_minimum` in `Space o o`.

### Structural syntax

| Key | Action |
| --- | --- |
| `Space x e` / `Space x s` | Expand / shrink syntax selection |
| `Space x p` / `Space x c` | Select syntax parent / first child |
| `Space x h` / `Space x l` | Select previous / next syntax sibling |
| `Space x o` | Open the immediate Tree-sitter document outline |
| `Space x x` | Toggle the syntax fold at the cursor |
| `Space x f` / `Space x u` | Fold / unfold all syntax regions in this pane |
| `Space x a f/c/p` | Select around the enclosing function / class / parameter |
| `Space x i f/c/p` | Select inside the enclosing function / class / parameter |
| `Space x a (/[/{/</"/'/\`` | Select around the matching delimiter pair; closing brackets are aliases |
| `Space x i (/[/{/</"/'/\`` | Select inside the matching delimiter pair; closing brackets are aliases |
| `Space x a m` / `Space x i m` | Select around / inside the closest enclosing delimiter pair |
| `Space x [ f/c/p` | Go to the previous function / class / parameter |
| `Space x ] f/c/p` | Go to the next function / class / parameter |

Structural expansion retains Tree-sitter's half-open bounds. Relationship
commands and `Space x a/i` text objects present those same bounds with the
block cursor on the last included character, matching ordinary Select mode;
yank, delete, change, and indentation still act on exactly the highlighted
syntax span.
Delimiter objects resolve through structural nodes in source languages. In
ordinary Markdown prose, where punctuation is not represented by delimiter
nodes, they use a balanced scan bounded to the enclosing Markdown syntax node;
escaped delimiters are ignored and injected code remains syntax-structural.

In Insert mode, Enter preserves the row's exact leading tabs/spaces and adds
at most one `tab_width`-sized space level when the syntax indentation query
requests it. On list items beginning with `-`, `*`, `+`, a decimal number, a
single letter, or a canonical uppercase Roman numeral followed by `.`, it
instead aligns the next line under the first content character, including at
nested indentation.
Set `editor.smart_newline` to `false` to preserve only the row's existing
leading indentation, without list alignment or an added syntax level.
Unsupported, malformed, oversized, and unterminated-final-line cases retain
the exact prefix and never block newline insertion. Syntax folds
are pane-local: two panes may collapse different regions of one shared buffer,
and any edit invalidates both panes' revision-scoped folds. A collapsed row is
marked by an accent-colored `▸` between its line number and the gutter rule;
its muted `… N lines` suffix reports how many complete rows are hidden.

Language servers may not create, rename, or delete files. A workspace edit
that asks for one has those operations reported and skipped, and files a
rename touches are opened as buffers rather than written behind your back.
Text edits must name absolute local files inside the project and exact,
forward-ordered character boundaries; malformed, remote, out-of-range, and
overlapping edits reject the complete multi-file change. Open target buffers
are guarded at the revisions from which a request was made, including targets
other than the command's source buffer. Versioned diagnostic publications are
likewise ignored after their open document advances.

### Commands

Pressing `:` opens the categorized command palette with every command, its
aliases, usage, and short description. Filtering searches canonical names,
aliases, descriptions, and categories. Commands whose active-buffer service
is unavailable remain discoverable but are dimmed with a reason; activating
one leaves the typed command and editor state intact. Use Up/Down to select a
result, Tab to complete it, Enter to run it, and Escape to close the palette.
Once a command with a path argument is selected, the rows become bounded
filesystem hints from the editor working directory (or from the absolute path
being typed). Directories appear first with a trailing separator; completing
one keeps the palette open for its children, while running `:open` on it opens
the editable directory explorer. A leading `~` means the user's home directory,
so paths such as `:open ~/.bashrc` and `:open ~/projects` work without shell
expansion. Dotfiles are offered when their name begins with `.` or hidden files
are enabled.

```text
:cd <path>               change the working directory; retarget an active explorer
:about                   show Runyte's logo, version, and getting-started guide
:tutorial [reset|sessions]
                        open or resume the interactive tutorial; reset starts
                        over and sessions opens its persistent-session lesson
:zen                     toggle a centered, maximized editable writing viewport
:fullscreen              toggle the active pane across the whole editor area, at its ordinary width
:close                  close the active buffer in place (aliases: c, buffer-close, bc, close-buffer, cb)
:close!                 discard unsaved text and close the active buffer (aliases: c!, buffer-close!, bc!)
:window-close           close the active pane, but not the last one (alias: wc)
:buffer-new             open a new scratch buffer in the current pane (alias: new)
:diff-disk              compare a fresh disk snapshot with the active file buffer
:diff-this              mark this buffer, or compare it with the one marked before it (aliases: difft, dt)
:diff-off               close the comparison this buffer is part of (alias: do)
:explorer [path]        open an editable directory explorer (alias: files)
:file-picker            fuzzy-find a file below the project root
:file-picker-directory  fuzzy-find below the active file/explorer directory
:fuzzy-grep             fuzzy-search contents below the project root
:fuzzy-grep-directory   fuzzy-search contents below the active file/explorer directory
:format                 format the active buffer (alias: fmt)
:git-blame              show live-buffer attribution for the primary line
:git-blame-file         open full-file live-buffer attribution
:git-branches           open the local branch list
:git-cancel             stop the active Git operation and reconcile mutations
:git-commit             write a message and commit what is staged
:git-diff               show the active file's unstaged diff
:git-diff-side-by-side  compare the active file's complete Git versions
:git-discard            throw away a file's uncommitted changes, after a confirmation
:git-index              review everything staged for the next commit
:git-log                open the Git log, or refresh it from its first page
:git-search-commits     fuzzy-search commits by message, ID, author, or date with a full-message preview
:git-refresh            re-read branch, changed files, and changed lines from Git
:git-stage              stage the active file, or every file selected in the list
:git-status             open the changed-file list
:git-unstage            unstage the active file, or every file selected in the list
:git-worktrees          open the repository worktree list
:grammar [runyte]       report the active Runyte editing grammar
:help [topic]           open the general manual, optionally at a named section (alias: ?)
:log-open               open the diagnostic log owned by the process that
                        holds this workspace
:lsp-restart [language] restart stopped language servers
:lsp-status             report language server state
:notifications          open retained notification history (alias: not)
:service-health         inspect syntax, LSP, providers, and helper health (alias: health)
:hsplit [path]          create a stacked split (alias: split)
:open <path>            open a file or directory in the active pane (aliases: e, edit)
:path                   show the active buffer's absolute path in a popup;
                        Tab offers copying it to the system clipboard or the
                        unnamed Runyte register
:detach                 disconnect this persistent TUI while retaining all editor state
:quit                   close the pane and its unique buffer, or stop safely from the last one (alias: q)
:quit!                  discard its unique buffer, or force quit from the last pane (alias: q!)
:quit-all               quit safely regardless of pane count, without ending terminals (alias: qa)
:quit-all!              discard buffer changes and quit, without ending terminals (alias: qa!)
:quit-here              quit and return the shell to the active directory (alias: qh)
:quit-here!             discard changes, quit, and return there (alias: qh!)
:reload                 reload the active file or refresh the active explorer or supported Git list
:resize-right +/- N     grow or shrink the pane at its right edge by N cells
:resize-left +/- N      grow or shrink the pane at its left edge by N cells
:resize-top +/- N       grow or shrink the pane at its top edge by N cells
:resize-bottom +/- N    grow or shrink the pane at its bottom edge by N cells
:outline                open the immediate Tree-sitter document outline
                        (alias: document-outline)
:config                 open the settings menu (alias: settings)
:terminal [command]     run a program in this pane, or $SHELL (aliases: t, term)
:terminal-file-directory [command]
                        run from the active file's parent
:terminal-directory-root [command]
                        run from the active explorer root
:terminal-selected-directory [command]
                        run from the selected directory entry
:terminal-session-directory <id|name>
                        run a shell from another terminal's safe directory
:terminals              list the running terminals and show one here
:terminal-show <id|name> show a terminal in this pane
:terminal-rename <name> name this pane's terminal
:terminal-output        copy this terminal's output into a read-only buffer
:terminal-send [id|name] send the selection, or the whole buffer, to a terminal
:theme [name]           choose a theme in the settings menu, or switch
                        straight to the named one
:vsplit [path]          create a side-by-side split
:write [path]           save, optionally choosing a path (aliases: w, save)
:write! [path]          save, replacing an existing file or one that changed
                        on disk (aliases: w!, save!)
:write-quit             save, then close the pane or quit from the last one (alias: wq)
:write-buffer-close     save and close the buffer in place (alias: wbc)
:session-list           open the session manager (persistent mode; alias: sl)
:session-attach WORKSPACE
                        attach to another workspace's persistent session
                        (alias: attach)
:session-start [WORKSPACE]
                        start a persistent session without switching
:session-stop [WORKSPACE]
                        stop a clean persistent session
:session-rename WORKSPACE NAME
                        rename a persistent session
```

The working directory starts at the directory where Runyte was launched.
`:cd <path>` changes it; relative paths are resolved from the current working
directory. When an explorer is active, `:cd` also retargets that explorer.
From a normal file buffer it leaves the file open. `Space e` opens the active
buffer's directory and selects that file, so Enter returns to its buffer;
`Space E` opens the working directory. A pathless buffer falls back to the
working directory. `Space / f` (`Space f`) always
opens the stable-project finder, while `Space / g` searches that root's
contents. `:file-picker-directory` and
`:fuzzy-grep-directory` search the active file's parent, the active explorer's
current root, or the working directory for a pathless/generated buffer.

### Change the shell directory on exit

Like Yazi, Runyte cannot change the working directory of the shell process
that launched it. Start it through a shell function that supplies
`--cwd-file`; then `:quit-here` or `:qh` exits with the normal unsaved-change
protection and changes the shell to the active explorer directory or the
active file's parent. A pathless view uses the last explorer directory visited
in that pane, then falls back to the working directory controlled by `:cd`.
`:quit-here!` or `:qh!` is the explicit force variant. Normal `:quit` from the
last pane and `:quit-all` leave the shell directory unchanged: use `:q`/`:qa`
when you want to stay where Runyte was launched, and `:qh` when you want the
shell to follow your navigation.

For Bash or Zsh, add this to the shell configuration:

```bash
function runyte() {
    local runyte_tmp runyte_cwd runyte_exit
    runyte_tmp="$(mktemp -t 'runyte-cwd.XXXXXX')" || return
    command runyte --cwd-file "$runyte_tmp" "$@"
    runyte_exit=$?
    if [ "$runyte_exit" -eq 0 ] && IFS= read -r -d '' runyte_cwd < "$runyte_tmp"; then
        [ -n "$runyte_cwd" ] && [ "$runyte_cwd" != "$PWD" ] && [ -d "$runyte_cwd" ] && builtin cd -- "$runyte_cwd"
    fi
    command rm -f -- "$runyte_tmp"
    return "$runyte_exit"
}
```

After reloading the shell configuration, invoke `runyte` normally. The
`--cwd-file` option is intended for shell integration; Runyte writes it only
after a successful `:quit-here` command. Without the wrapper, `:quit-here`
refuses to exit and explains how to enable the handoff instead of silently
acting like plain `:quit`.
Session-management commands such as `--session-list` accept the option but leave the
file untouched, so they can be invoked through the same shell function.

The wrapper works the same way against a persistent host. `:quit-here` runs in
the host, which reports the directory it chose while the attached client writes
the file — so the same wrapper serves both modes, and the directory follows you
across a workspace switch. Because the capability belongs to the client rather
than the host, a client launched without the wrapper still gets the usual
refusal even when an earlier one had it. In persistent mode `:quit-here` stops
the session after the same safety checks as `:quit`; use `:detach` when the host
should remain running.

## Diagnostics and logging

Four surfaces answer four different questions. The interaction line reports
what the last command did. `:notifications` keeps bounded workspace-lifetime
feedback in memory. `:service-health` describes optional services right now.
The **diagnostic log** is the durable one: a small local file that outlives the
process, so a failure can still be read after Runyte is gone. It is not a
second notification system and not an audit trail — an actionable failure
still reaches you through the ordinary surfaces whether or not a record was
written.

### Who owns the log

Log ownership follows editor-state ownership.

| Mode | Owner | File |
| --- | --- | --- |
| Standalone | the TUI process | `.runyte/standalone-<pid>.log` |
| Persistent | the host process | `.runyte/host.log` |

A standalone name carries the process ID because more than one standalone
editor may open the same workspace; two of them can never write or rotate the
same file. A host has one canonical name because exactly one host serves a
workspace, and it keeps recording while no TUI is attached.

A client never appends to a host's log and never forwards records over the
local protocol: transport diagnostics must not depend on the transport being
healthy. What the host observes about clients — attachment, detachment,
refusal, disconnection, and rejected frames — is recorded by the host. Failures
confined to a client's own input, rendering, or terminal setup are printed to
stderr after the terminal has been restored, and are not claimed by the host
log.

Logs live under the configured runtime state boundary, normally `.runyte/`, and
never under a Git-tracked directory.

### Reading the log

`:log-open` opens the log of the process that owns this workspace as an
ordinary read-only `[log]` buffer, so it is searchable, splittable, and
scrollable with the usual keys. In persistent mode that is the host's
`host.log`; no client-side trace is opened or aggregated. `:service-health`
names the owner role, active level, resolved path, and any logger failure.

Each record is one line: an RFC 3339 timestamp, the level, the owning role and
PID, the abbreviated workspace ID, the subsystem, the message, and any
structured `key=value` context.

```
2026-08-27T12:34:56.789+02:00 WARN  host[8123] ws=a1b2c3d4 lsp: language server stopped: exited with 1 language=rust
```

### Levels and startup controls

The default level records warnings and errors, so an unexpected first failure
does not have to be reproduced, while routine operation produces no
high-volume trace. Each `-v` raises the level and the cap is trace:

| Flags | Level |
| --- | --- |
| *(none)* | warning |
| `-v` | info |
| `-vv` | debug |
| `-vvv` and beyond | trace |

`--log PATH` selects an explicit destination. Failing to honour it is a startup
error, because silently choosing another file would make the requested capture
misleading. On Unix, a path already owned by another running Runyte process is
refused after a two-second handover window; choose a different path or let the
first process exit. An unwritable *default* destination only degrades logging:
editing continues, a persistent host still serves, and the failure appears on
stderr, as a notification, and in `:service-health`. If a destination stops
accepting writes after startup, editing likewise continues and one warning is
retained in `:notifications` as well as the standing failure in
`:service-health`.

In persistent mode, verbosity and destination are properties of host startup.
`--serve`, `--session-start`, `--session-restart`, and the launch that creates
a missing host pass them to that host. Attaching to an already-running host
does not change its logger; supplying `-v` or `--log` there reports that the
session kept its own configuration and that a restart is required:

```sh
runyte --session-restart -vv     # the only way to change a running host's logging
```

There is no runtime log-level command and no protocol message for logging.

### What is recorded, and what never is

Recorded: process startup, version, role, workspace identity, and orderly
shutdown; session publication, idle retirement, signal termination, and forced
termination; client attachment, detachment, refusal, closure, a client that
stops reading, and malformed or truncated frames; language servers becoming
ready, restarting, and stopping; terminal sessions that fail to start and
children that exit; Git failures already converted into typed results;
background services whose channel closes; and panics, including the thread,
location, message, and backtrace when backtraces are enabled.

Not yet recorded, though the surrounding events are: filesystem-watcher
lifecycle beyond its channel closing, and a host restart as distinct from a
start followed by a stop.

Never recorded: routine keystrokes, rendered frames, successful editor
commands, buffer edits, and complete language-server request or response
bodies. Default and verbose logging never contains buffer text, selections,
clipboard contents, typed or pasted text, terminal contents, credentials,
environment-variable values, unrestricted subprocess output, or full LSP JSON.

Two records are deliberately thinner than the message the person sees. A
failed Git operation records the refusal and its exit status but not the
argument vector or Git's stderr, because a failing commit's argument vector
holds the message that was just typed. A stopped language server records the
language but not the composed reason, because a server closed by its own
process carries its stderr tail in that text. Both remain available in full
through `:notifications`, `:lsp-status`, and the interaction line.

Records **do** contain local paths and process metadata, because those are what
identify a failing local operation. Review a log before sharing it.

### Bounds

At most 4 MiB is kept in the active file, with one previous 4 MiB file beside
it, named by appending `.1`. Rotation is owned by the same process that owns
the file and happens both while a host runs and at startup when it inherits a
full file, so a long-lived or often-restarted host cannot grow without bound.

Standalone names are unique, but every launch would otherwise leave another
file behind. Before opening its own default log, a standalone process keeps the
four newest logs belonging to exited standalone processes and removes older
ones together with their `.1` files. Logs belonging to live processes are
never pruned.

Producers never wait for disk. Records go through a bounded queue to a single
background writer, and are dropped rather than delaying input, rendering,
local-protocol handling, or service events; a later record summarizes how many
were lost. Shutdown and panic paths flush within a bounded budget and never
wait indefinitely.

## Configuration

The default path is:

```text
$XDG_CONFIG_HOME/runyte/config.yaml
```

or `~/.config/runyte/config.yaml` when `XDG_CONFIG_HOME` is unset. Use
`--config <path>` to load another file. A relative path is anchored to the
directory where Runyte was launched, even when workspace initialization later
enters another directory. All fields are optional.

Open the registry-backed `[config]` buffer with `Space o o`, `:config`, or
`:settings`. It is a left-aligned, read-only, searchable document: normal
motions, selections, splits, and keybindings continue to work. Its
80-character rows contain setting, description, and saved-value columns, with
long content wrapped onto physical rows that retain the setting's identity.
Enter anywhere on a setting's rows opens its value popup. Finite grammar,
boolean, and theme values use a choice list; numeric values use a typed popup
that shows and enforces the registry's minimum and maximum. Moving through an
immediate choice previews it, Escape rolls the preview back, and Enter
atomically patches the loaded YAML file while preserving comments, ordering,
and unknown fields.
`Space o t` and a bare `:theme` open the theme choices directly. That list is
the one long enough to be worth reading in halves, so `Tab` narrows it to the
dark themes, then to the light ones, then back to every theme; the popup title
names the group it is showing. The grouping comes from each theme's own
background, so a theme declared in `themes:` is sorted into the same halves as
a built-in one. Typing still filters within whichever group is shown. Changing
`lsp.enable` is saved but explicitly reports that a restart is required; the
menu never presents it as a live server transition.
Changing `workspace.mode` is also restart-required because the standalone or
persistent launch path is selected before the editor application starts; the
saved choice applies to future bare launches.
`workspace.idle_retirement_minutes` is in the same menu but applies at once: a
persistent host reads it each time it considers retiring, so a shorter or
longer interval takes effect without restarting the host it governs.
YAML features that cannot be patched losslessly are rejected with the file
left untouched. A failed save also rolls back any live preview while keeping
the choice popup open for correction or retry.

`Space o s` or `:service-health` opens a read-only snapshot of syntax registry
failures, the active document's LSP configuration and attachment, and the
diagnostic log's owner role, level, resolved path, and any failure. The
report probes paths only and remains useful when every optional service is
absent. In persistent mode its `log` row describes the host that owns the
workspace, not the client process that opened the report.

```yaml
editor:
  grammar: runyte # `helix` is accepted as a compatibility alias
  line_numbers: true
  tab_width: 4
  smart_newline: true # add syntax indentation and align list continuations
  scroll_offset: 3
  motion_repeat_multiplier: 2 # held cursor motions; 1 retains terminal/Helix speed
  show_hidden_files: false # explorer, file picker, and workspace search; . toggles it in an explorer
  soft_wrap: false
  render_whitespace: false # show · for spaces, → for tabs, and ↵ for line endings
  zen_width: 100 # maximum text width while :zen is active; editable in :config's popup
  hard_wrap_width: 80 # width for Space p w and Space p r; editable in :config's popup
  trim_trailing_whitespace: true # remove spaces and tabs at line ends on save
  mouse: true # set false to retain the terminal's native text selection; restart required
  word_completion: true # suggest words already open elsewhere in the workspace
  word_completion_minimum: 3 # prefix length before word candidates appear
  fast_pane_keys: false # Ctrl-h/j/k/l move between panes without the Ctrl-w prefix
  command_mode_dim: true # gray out every pane's text while a command prompt is open

workspace:
  state: .runyte # `root` is accepted as a compatibility alias
  mode: standalone # `persistent` changes future bare launches; restart required

notifications:
  history_limit: 50 # newest workspace-lifetime notifications kept in memory

# Optional. Leave it out to use the light theme.
theme: gruvbox
```

`editor.motion_repeat_multiplier` applies only to automatically repeated
single-key movement commands, including the arrow keys and `h`, `j`, `k`, and
`l`; an ordinary key press still runs once. Runyte requests enhanced keyboard
event reporting from terminals that support it so held presses can be
distinguished from fresh presses. On macOS it requests only unambiguous key
codes and keeps repeat detection on the legacy cadence path, avoiding terminal
event streams that have reported ordinary keys as repeats there. For terminals
that report auto-repeat as ordinary presses, Runyte recognizes the long initial
delay followed by the regular held-key cadence and applies the same multiplier.

Git-derived views and tracked-file gutters refresh every five seconds while
relevant state is visible. Set `git.refresh_interval_seconds` to another
interval, or to `0` to disable the timer; `:git-refresh` remains available.

`workspace.state` is where a workspace keeps its local runtime state. It may be
absolute or relative to the workspace directory. Runyte finds that directory by
walking upward from the launch directory, first looking for a Git root and
then, only when there is no Git root, for the configured relative state
directory (`.runyte/` by default). If neither marker exists, Runyte asks where
project data should live and creates nothing until that location is explicitly
confirmed. Confirming it creates the state directory there, so the same
workspace is found without asking again on later launches. Because discovery
walks upward, confirming your home directory makes every directory below it
with no Git repository and no state directory of its own part of that one
workspace; the prompt says so before asking to confirm that particular
location.

The key was previously spelled `workspace.root`, which read as the workspace's
own root while naming the state directory nested inside it. The old spelling is
still accepted.


Built-in themes are `dark`, `light`, `base16`, `paper`, `gruvbox`,
`atom-one-light`, `github-light`, all four
Catppuccin flavours (`latte`, `frappe`, `macchiato`, and `mocha`), and the six
Everforest variants (`everforest-dark-hard`, `everforest-dark-medium`,
`everforest-dark-soft`, `everforest-light-hard`, `everforest-light-medium`,
and `everforest-light-soft`), plus `nordfox`, `nordfox-warm`, `terafox`, and
`terafox-soft`.
The Fox themes use the canonical palettes from
[Nightfox](https://github.com/EdenEast/nightfox.nvim): `nordfox` is a cool
Nord-derived dark theme, `nordfox-warm` keeps that base while using brighter
dimmed text with pink secondary and yellow primary selections, and `terafox`
uses deep blue-green backgrounds and warm accents. `terafox-soft` is Runyte's
own variant rather than an upstream palette: it is `terafox` with the ordinary
text brought down from 13.1:1 against the background to 8.6:1, for reading at
length without the glare. Its identifiers, operators and punctuation move by the
same amount so Terafox's own ordering survives, and everything else — the
background, the hued syntax colours, the accents, the selections, and the diff
rows — is Terafox unchanged. `atom-one-light` follows
Atom's official
[One Light UI](https://github.com/atom/one-light-ui) and companion
[One Light syntax](https://github.com/atom/one-light-syntax) palettes.
`github-light` follows projekt0n's
[GitHub Theme for Neovim](https://github.com/projekt0n/github-nvim-theme).
The Zenbones collection contributes `zenbones`, `zenwritten`, `neobones`,
`rosebones`, `forestbones`, `tokyobones`, and `seoulbones` in `-light` and
`-dark` variants, plus `vimbones-light`, `nordbones-dark`, `duckbones-dark`,
`zenburned-dark`, and `kanagawabones-dark`. These use the concrete palettes
and generated highlight colors from
[Zenbones](https://github.com/zenbones-theme/zenbones.nvim). Its dynamic
`randombones` selector is intentionally not a Runyte theme.
`nordbones-dark-soft` is Runyte's own variant rather than an upstream palette:
it is `nordbones-dark` with the ordinary text brought down from 10.6:1 against
the background to 7:1, for reading at length without the glare. Everything
else — the background, the accents, the selections, and the diff rows — is
Nordbones unchanged.
`dark` and `light` are the neutral pair: no palette identity of
their own, just a legible default for each kind of terminal. Switch directly
with a command such as `:theme atom-one-light`, `:theme mocha`, or
`:theme everforest-dark-medium`, or run `:theme` with no name to choose one in
the settings menu. The same theme choice is available from the `theme` row in
`:config`.

Whichever theme is selected is written to `theme:` in the configuration file
and used the next time Runyte starts. With no configured theme, Runyte starts
in `light`. A custom theme can be declared in the same file:

```yaml
theme: midnight

themes:
  midnight:
    background: "#10131a"
    foreground: "#d8dee9"
    muted: "#65737e"
    whitespace: "#292c33"
    accent: "#88c0d0"
    cursor_normal: "#ff5555"
    cursor_insert: "#ff79c6"
    cursor_replace: "#39ff14"
    cursor_select: "#ffb86c"
    cursor_command: "#bd93f9"
    directory: "#8be9fd"
    selection: "#2e3440"
    selection_primary: "#694b37"
    fuzzy_match_secondary: "#2e3440"
    fuzzy_match_primary: "#694b37"
    error: "#bf616a"
    warning: "#d08770"
    info: "#a3be8c"
    jump_label_immediate: "#ff5555"
    jump_label_primary: "#5fd7e7"
    jump_label_secondary: "#4ab7c6"
    change_added: "#a3be8c"
    change_modified: "#ebcb8b"
    change_removed: "#bf616a"
    diff_added: "#1d2a20"
    diff_removed: "#2c1c1f"
    diff_changed: "#2b2519"
    syntax:
      comment: "#65737e"
      keyword: "#b48ead"
      markup.heading: "#88c0d0"
      markup.italic: "#b48ead"
      markup.raw: "#a3be8c"
      string: "#a3be8c"
```

Markdown uses the semantic syntax scopes `markup.heading`, `markup.bold`,
`markup.italic`, `markup.link.text`, `markup.link.url`, `markup.list`,
`markup.quote`, and `markup.raw`. Bundled themes assign all of them colours;
custom themes may override any subset, and omitted scopes use the theme's
foreground like every other omitted syntax scope.

`whitespace` colours the display-only `·`, `→`, and `↵` markers. When a custom
theme omits it, Runyte derives a very dim colour one small step away from that
theme's `background`; a theme using the terminal's `reset` background falls
back to `muted` because its actual ground is unknown.

The same `jump_text_muted` colour grays out every pane while a command prompt
is open — `:` and the search, rename, and other text-entry prompts alike, since
all of them take the keyboard away from the panes. Pending keys such as `g` and
`Space` are not a mode and change nothing. Carets keep their mode colour so
Command stays identifiable, and the prompt and its completion list are never
dimmed. Set `editor.command_mode_dim: false` to keep every pane at its ordinary
colours instead.

While `gw` is waiting, ordinary text in the active pane uses the optional
`jump_text_muted` colour so the labels stand forward; themes that omit it use
`muted`. The built-in `light` and `paper` themes use lighter jump-only grays
without changing their normal comments or UI. Nearby targets receive a
one-key label in `jump_label_immediate`; farther targets receive two characters
in `jump_label_primary` and `jump_label_secondary`. After the first key of a
two-key label, only matching second keys remain and they move to the target cell
in `jump_label_immediate`. Omitting `jump_label_immediate` uses the theme's
`error` colour, preserving older custom themes. Built-in themes use one
neon-cyan hue for both two-key characters: the second is darker on dark
backgrounds and lighter on light backgrounds. The Zenbones light palettes use
a darker pair, while the mid-gray `seoulbones-dark` and `zenburned-dark` use a
brighter pair, so both characters retain text contrast across those palettes'
different background range.

The visual presentation of `gw` is inspired by
[`smoka7/hop.nvim`](https://github.com/smoka7/hop.nvim); Runyte's
implementation and viewport projection are independent.

`change_added`, `change_modified`, and `change_removed` colour the Git gutter,
and the first and last also colour added and removed lines in a diff buffer and
the two count columns in the changed-file list, so added text is the same green
wherever you meet it. A theme that omits them uses
the terminal's own green, yellow, and red, which are legible against whatever
palette the terminal already has.

`diff_added`, `diff_removed`, and `diff_changed` fill whole lines in a
side-by-side comparison, so unlike the three above they have to be tints of the
background rather than the strong colours a one-cell mark can use. A theme that
omits them leaves those lines unfilled and lets the gutter marks carry the
comparison on their own, so the feature still works on a theme written before
it existed.

`error`, `warning`, and `info` colour notification headings and unread status
counts. Custom themes that omit `warning` use `change_modified` (then terminal
yellow), while omitted `info` uses `change_added` (then terminal green).

`cursor_normal`, `cursor_insert`, `cursor_replace`, `cursor_select`, and
`cursor_command` colour both carets and the global status line's mode label in
Normal, Insert, Replace, Select, and Command modes respectively. When omitted
they fall back to `accent`, `error`, Runyte's neon Replace accent, `warning`,
and `info`. The Replace fallback is saturated green against both dark and
light grounds; it switches to neon magenta when another resolved mode colour
is green. The built-in themes use blue for Normal, red for Insert, neon green
for Replace, orange for Select, and purple for Command.

`selection` colours secondary ranges in a multi-selection.
`selection_primary` colours the primary range and ordinary Select-mode ranges;
it falls back to `selection` when omitted. Runyte's original built-in themes
pair a cool secondary selection with a warm primary selection; imported themes
may instead preserve their upstream Visual and Search backgrounds, as the
Zenbones variants do.

`fuzzy_match_secondary` colours the individual characters of a non-contiguous
fuzzy-grep match, while `fuzzy_match_primary` colours a direct, contiguous
substring. They fall back to `selection` and `selection_primary` respectively,
so existing and built-in themes use the same blue/orange grammar as `Space s`.

`directory` colours directory entries in explorer buffers; ordinary files
keep the theme foreground. It falls back to `accent` when omitted.
The built-in light palettes use a dark blue and the built-in dark palettes use
a lighter palette blue.

The active pane keeps `background`; inactive panes use a derived ground halfway
toward the overlay ground. Every floating popup — pickers, choice lists, the
command palette, action menus, key hints, and the language-server popups —
paints on a ground one step off `background`: lighter on a dark theme, darker
on a light one. That ground is derived from the theme rather than named in it,
so a theme declared in `themes:` separates its overlays exactly as a built-in
one does and none can forget to. The step is small enough that both grounds
still read as the same colour. A theme whose `background` is `reset` leaves the
ground to the terminal, which cannot be stepped off, so its overlays keep the
one background.

Syntax scopes are `attribute`, `comment`, `constant`, `constructor`,
`function`, `keyword`, `label`, `namespace`, `number`, `operator`,
`property`, `punctuation`, `string`, `tag`, `type`, and `variable`. Any scope
left out falls back to the theme foreground, and an unknown scope name is a
configuration error rather than a silent no-op. Tree-sitter captures map to
the most specific scope that prefixes them, so `keyword.control.return` is
themed by `keyword`.

Colors accept `#rrggbb` or terminal color names such as `black`, `red`,
`green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `grey`, and `dark-grey`.
See [config.example.yaml](../config.example.yaml) for a complete starting point.

## Project layout

```text
src/
  app.rs          editor state, shared application types, and startup coordination
  app/            editor-level Git, workspace, pane, input, editing, terminal,
                  search, syntax, file, picker, settings, and LSP workflows
  buffer.rs       rope-backed buffers, file I/O, and transactional undo
  clipboard.rs    testable operating-system clipboard adapters
  text.rs         rope storage, character offsets, and transactions
  selection.rs    multi-range selections over character offsets
  syntax/         tree-sitter highlighting and the bundled grammar table
  lsp/            asynchronous language-server transport and typed events
  git/            bounded Git commands, projections, diffs, and status tracking
  terminal/       PTYs, emulation, bounded scrollback, and terminal sessions
  workspace/      persistent host state, attachment, and service lifecycle
  protocol/       private versioned DTOs for bundled local clients (Unix)
  command.rs      editor commands and shared display metadata
  headless.rs     frontend-independent semantic editor test facade
  snapshot.rs     owned presentation-neutral editor and overlay snapshots
  config.rs       YAML settings and theme resolution
  directory_buffer.rs
                  editable directory projections and hidden entry identities
  fs_plan.rs      confirmed filesystem plans, conflicts, trash, and application
  diff.rs         line correspondence shared by Git and side-by-side comparison
  diff_view.rs    live paired-buffer comparison state and aligned scrolling
  hash.rs         stable content hashing used by buffers, Git, and transport
  path_safety.rs  canonical project-boundary checks
  external_open.rs
                  binary detection and the remembered programs that open them
  file_picker.rs  fuzzy matching, ignore-aware discovery, and text previews
  picker.rs       shared presentation-neutral filterable result state
  help.rs         per-view prose and the registry-derived help document
  jump_labels.rs  proximity-ranked one- and two-key `gw` labels and narrowing
  key_hints.rs    registry-backed key discovery state
  keymap.rs       declarative bindings and sequence lookup
  layout.rs       recursive split tree
  notification.rs bounded workspace-lifetime history and its buffer document
  ui.rs           Ratatui widgets and editor frame composition
  wrap.rs         Unicode cell-aware visual-line and soft-wrap geometry
  main.rs         CLI, event loop, and Crossterm terminal lifecycle
```

The compatibility status for implemented, deviating, and removed Helix
bindings is tracked in `context/reference/helix-keymap-v1.md` in the
repository.

The UI is rendered into Ratatui's in-memory cell buffer. Ratatui compares each
completed frame with the previous one and sends only changed cells through its
Crossterm backend. The event loop blocks while idle, so unchanged screens are
neither reconstructed nor written to the terminal.

## License

Runyte is licensed under the [Mozilla Public License 2.0](../LICENSE).

For official binary releases, the corresponding source code is available from
the [Runyte repository](https://github.com/runyte/runyte). Releases from
0.1.0 onward have a matching release tag. Third-party dependencies and assets
remain subject to their own licenses.
