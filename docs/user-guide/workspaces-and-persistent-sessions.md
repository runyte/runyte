## Workspaces and persistent sessions

Runyte runs in one of three modes. The status line names the one in use, and a
session keeps its mode until it ends.

| Mode | Flag | Workspace | After the TUI exits |
| --- | --- | --- | --- |
| **editor** | `--editor`, or the `runed` command | None: edits files and directories anywhere | Gone |
| **ide** (default) | `--ide` | Yes, with Git, language servers, MCP, plugins, and terminals | Gone |
| **mux** (`ide+mux`) | `--mux`, `-a` | Yes, as in ide, kept by a local host process, the **persistent session** | Kept: open and unsaved buffers, selections, registers, syntax state, diagnostics, Git projections, language-server processes, and terminal sessions |

The `mode` setting chooses the mode for launches that give no mode option; it
is `ide` unless set otherwise. The launch directory never changes the mode.

A **workspace** is one project directory and its editor scope: a Git,
Jujutsu, or Subversion working copy, or a directory holding the workspace state
directory (`.runyte/`), found by walking up from the launch directory. ide and
mux need one. Outside a workspace, an ide launch makes the launch directory one;
mux refuses, and so does ide in the directories listed under
[Finding the workspace](#starting-a-workspace), with a message naming
`runyte --init DIRECTORY`.

Persistent sessions work on Unix and Windows; see
[Windows support](install-and-run.md#windows-support) for the Windows differences.

### Starting a workspace

| Command | Effect |
| --- | --- |
| `runyte` | Open the workspace discovered from the current directory, in the mode set by `mode` |
| `runyte --mux`, `runyte -a` | Attach to the current project's persistent session, starting it if needed |
| `runyte -a WORKSPACE` | Attach to a named session from any directory, starting it if needed |
| `runyte --ide` | ide mode, overriding the `mode` setting |
| `runyte DIRECTORY` | Open that directory in the workspace discovered from the current directory |
| `runyte --init /path/to/project` | Make that exact directory a workspace and exit |
| `runyte --serve` | Run the host in the foreground |

**Finding the workspace.** Runyte walks up from the launch directory to the
nearest Git (`.git`), Jujutsu (`.jj`), or Subversion (`.svn`) root, or else to a
workspace state directory (`.runyte/`). A working copy wins over a nearer state
directory. When neither exists, an ide launch makes the launch directory the
workspace, as Helix uses its working directory: it creates `.runyte/` there,
says so in a notification, and later launches from it or from a directory below
it find the same workspace. See
[Where workspace state lives](configuration.md#where-workspace-state-lives).

Some directories are never made a workspace this way, because a `.runyte/` in
them would become the workspace of every later launch from a directory below
with no repository:

- your home directory;
- the filesystem root;
- the temporary directory;
- on Unix, a directory other accounts can write to, or one another account
  owns.

A launch naming a file outside the launch directory, such as
`runyte /etc/hosts` from `~/Downloads`, is not made one either: it is about
that file, not about where the shell happened to be. These launches refuse;
run `runyte --init DIRECTORY` to make a directory a workspace deliberately, or
use `runyte --editor`.

A launch that fails after making its workspace, for example because it has no
terminal, leaves the `.runyte/` it created. mux launches, `--wait`, and session commands never create a
workspace: run an ide launch or `--init` there first.

**`--init DIRECTORY`** creates the configured state directory (`.runyte/` by
default) in exactly that directory, prints how to open it, and exits without
starting the editor:

```
initialized a workspace in /path/to/project
open it with: runyte /path/to/project   or   runyte --mux /path/to/project
```

- An existing state directory is used as is, never reset or removed, and the
  output says it is already a workspace.
- It picks the named directory even when an ancestor has its own state
  directory.
- It refuses a state directory that would overlap your configuration or cache
  directories.

**`mode: mux`** makes a bare `runyte` attach like `runyte -a`.

- Launches naming a target — a file or a directory, including `runyte .` — run
  in ide mode in this process, so their relative paths and
  `+LINE[:COLUMN]` positions keep ordinary meaning.
- `--mux` reads its argument as a workspace, not a file.

**`runyte -a WORKSPACE`** uses the same selector as the lifecycle commands (see
[Selecting a session](workspaces-and-persistent-sessions.md#selecting-a-session)).

- A session that is not running is started first.
- An existing directory unknown to the catalog names that exact directory:
  Runyte creates its state directory if needed and starts its session.
  `:session-attach DIRECTORY` and the explorer's `Tab s` do the same.
- A bare `runyte -a` uses the workspace of the current directory, and refuses
  when there is none.

**Only one interactive TUI** may be attached at a time. Separate control
connections can still manage the host.

### Editor mode

**Editor mode** is Runyte as a simple editor for a configuration file or a
system directory: one process with no workspace. Start it with
`runyte --editor [FILE or DIRECTORY]`, or with `runed`, which is the same
program under another name and is always editor mode, whatever the `mode`
setting says. `runed` rejects `--ide`, `--mux`, and the other options that
would choose a mode or a workspace.

The installer puts `runed` beside `runyte` as a link, and the release archives
include it (a copy, `runed.exe`, in the Windows zip). After `cargo install`,
add it yourself, for example `ln -s runyte ~/.cargo/bin/runed`.

**What editor mode keeps:** editing, keys, your configuration and theme,
syntax highlighting, undo, registers and the clipboard, safe saves, splits,
help, `:open` with path completion, the directory tree, and the editable
explorer with reviewed filesystem plans.

**What it leaves out entirely:** the `.runyte/` directory, the diagnostic log
(unless `--log PATH` is given), the recent-workspace list, persistent sessions,
Git (no repository discovery, status-line branch, gutter marks, or Git views,
even for a file inside a checkout), language servers, MCP, plugins, and
integrated terminals. Their commands report `not available in editor mode`,
and help and key hints grey them out. Pasting an image is refused too, because
pasted images live in `.runyte/`. A session in editor mode stays in it; start
`runyte` again for a workspace.

**Search covers the directory in front of you.** `Space f`, `Space / f`, the
all-files Finder, and the project content searches (`Space / s`, `Space / /`,
`:fuzzy-grep`) search below the active file's or explorer's directory instead
of a project root, and their titles and prompts name that directory, as in
`search /etc/nginx: `. Because that directory may be anywhere, these searches
are contained:

- They stay on the filesystem they start on, so mounts such as `/proc` below
  `/` are not entered.
- They stop after reading 20,000 directory entries, hidden and ignored ones
  included, and say the result is limited.
- They refuse to start at `/` or inside a virtual filesystem such as `/proc`,
  `/sys`, or `/dev`; open a narrower directory instead.

`g f` resolves relative paths beside the active file only.

**The status line** reads `editor` where it otherwise reads `ide` or
`ide+mux`, labels the directory `Directory:` rather than `Workspace:`, and
shows no Git information.

**Running as root,** Runyte runs only in editor mode: `--ide`, `--mux`,
`--wait`, `--serve`, `--init`, and a configured `ide` or `mux` mode refuse and
point to `runed` and `sudoedit`, because they would leave root-owned runtime state in a
workspace another account owns.

#### Editing system files with sudo

1. Set `SUDO_EDITOR` to `runed` in your shell profile:

   ```sh
   export SUDO_EDITOR=runed
   ```

   Use the installed `runed` on `PATH` rather than a path into a build
   directory such as `target/release`, which breaks after a clean or during a
   rebuild.
2. Edit single files with `sudoedit /etc/fstab`. sudo copies the file to a
   temporary path, runs Runyte as you with your own configuration and theme,
   and copies the result back as root after you quit. Quitting without saving
   leaves the file unchanged. Runyte itself never runs as root.
3. Use `sudo runed /etc` only for what `sudoedit` cannot do: renaming, moving,
   or deleting files in system directories through the explorer. Runyte then
   runs as root, the filesystem plan says `as root` in its title, and deletions
   go to root's trash (or the mount's trash directory), not yours. Runyte finds
   its configuration through `XDG_CONFIG_HOME` and `HOME`, so whose
   configuration applies depends on the sudoers environment policy; with the
   common defaults it is root's.

### Quitting and detaching

| Command | ide | mux |
| --- | --- | --- |
| `:quit` from the last pane | Exits | Stops a clean session and returns the TUI to the previously visited running session |
| `:quit-all` | Exits | The same, whatever the pane count |
| `:quit-here` | Exits and changes the shell directory, through the wrapper | The same; without the wrapper both refuse. See [Change the shell directory on exit](workspaces-and-persistent-sessions.md#change-the-shell-directory-on-exit) |
| `:detach` | — | Disconnects the TUI at once, keeping every pane, buffer, unsaved edit, and terminal in the host |

- If the previous session is unavailable, Runyte tries other running sessions
  in recent-visit order, skipping occupied or incompatible hosts, and returns
  to the shell when none can take the TUI.
- Quitting refuses unsaved buffers and live terminal children. A `!` form may
  discard unsaved buffers but never ends a terminal.
- `:detach` needs no force form, because it discards nothing.
- To stop a different session, use `runyte --session-stop` or
  `:session-stop`.

### Session keys and commands

| Key or command | Action |
| --- | --- |
| `Space Space`, `:session-list` (`:sl`) | Open the session manager |
| `Space 1`–`Space 9` | Attach directly to the numbered running session |
| `Shift-Left` / `Shift-Right` | Visit the previous / next running session in manager order |
| `Ctrl-w a` | Alternate between the last two attachments |
| `:session-attach WORKSPACE` (`:attach`) | Attach, starting a stopped session. Any existing directory becomes a workspace if needed. |
| `:session-stop [WORKSPACE]` | Stop a session without switching |
| `:session-rename WORKSPACE NAME` | Rename a session |
| `:session-clean` | Clean verified stopped history (Windows) |
| `:detach` | Disconnect this TUI, leaving the session running |

`Space 1`–`Space 9` work in Normal and Select modes, follow `keys.leader`, and can be rebound as `session-1`
through `session-9`. The key-hint popup groups them into one `Space 1-9` row;
individually remapped ones are listed separately.

**Stopping refuses** while the target owns protected buffers, waiters, or live
terminal children. Switching away is always safe, because the old host keeps
them.

**In ide mode** (Unix) there is no host, so the session namespace is
inert rather than a set of commands that each refuse. `Space Space` and
`Space 1`–`Space 9` are greyed in the key hints, `:session-list`,
`:session-attach`, `:session-stop`, and `:session-rename` are greyed in the
command palette, and invoking one answers `needs mux mode`.

### The session manager

`Space Space` or `:session-list` (`:sl`) opens a filterable list of running
and recently visited sessions, on Unix and Windows.

```text
  No. Name                    Branch            Path                             Last active  Status
  1   main                    main              ~/code/runyte                   3h ago
  2 * runyte-dev              dev               ~/code/runyte-dev               0min ago
  3   Brain                   -                 ~/Brain                         12days ago
  4   runyte.github.io        main              ~/code/runyte.github.io         5days ago     QUIET
  5   runyte-enh-render-space enh/render-space  ~/code/runyte-enh-render-space  1min ago
```

The title reads `Sessions · Enter open · Tab actions · Esc close`.

#### Manager keys

| Key | Action |
| --- | --- |
| Enter | Attach to the selected session in this TUI, starting it if necessary |
| `1`–`9` (empty filter) | Attach to that numbered session |
| `Tab` | The action menu |
| `Space` (empty filter), Escape, `Ctrl-c` | Close |
| `Ctrl-o` | **Open directory…** |
| `Ctrl-e` | **Open destinations** of the selected running session |
| `Ctrl-g` | **Git worktrees** |
| `Ctrl-n` / `Ctrl-p`, `Ctrl-d` / `Ctrl-u`, Home / End | Move and page |
| `Ctrl-t` | Toggle the preview |
| Delete | Clear the filter |

- `Space Space` is the complete binding, not a prefix. Once the filter has
  text, `Space` is filter text and only Escape or `Ctrl-c` closes.
- Digits are shortcuts only while the filter is empty, because default names
  such as `runyte-2` and many paths contain digits. Clearing the filter
  (Delete, or Backspace to empty) arms them again. A name or path starting with
  a digit therefore cannot be filtered by that first character; type a later
  part.
- `Space Space 1` reaches the first session in one gesture.

**The key legend** is pinned, dimmed, to the bottom. It names only keys that
act now: `1-9 attach` while the filter is empty in a persistent editor, the
`Ctrl-o`, `Ctrl-e`, and `Ctrl-g` chords while they can act, moving and paging,
Home/End, `Ctrl-t`, and Delete while there is a filter. Rows take priority: in
a short manager the legend first loses its spacing line, then disappears,
before any row is hidden.

**The Tab menu** lists the selected row's actions first, then, under a
**Manager** heading, the manager's own actions with their direct keys:

| Row | Actions |
| --- | --- |
| Running | Open, Rename, Renumber, Close, Force close |
| Stopped | Open, Rename, Forget |
| Manager | Open directory… (`Ctrl-o`), Open destinations (`Ctrl-e`, running row only), Git worktrees (`Ctrl-g`) |

- Open is the same as Enter.
- Close stops the host and keeps the workspace as a stopped row. Nothing
  below the session is touched, because a session is the only level that
  means nothing on its own.
- Forget removes only the history record behind a stopped row. Nothing in the
  project is touched; naming the directory again starts a host there and lists
  it again.
- An entry that cannot act here, such as Git worktrees outside a repository,
  is dimmed with the reason and refuses without closing the manager.

#### Columns

| Column | Shows |
| --- | --- |
| `No.` | The digit that attaches to the row; `*` marks the current session |
| `Name` | The session name |
| `Branch` | The checked-out branch, or `-` outside a Git working tree |
| `Path` | The workspace directory, with `~` for your home; the preview has the full path |
| `Last active` | How long since the last visit |
| `Status` | Plugin work or terminal-output state |

- Columns are padded to the widest value or heading so they line up. Headings
  stay above the rows and are not filtered or selectable.
- The branch is read from the directory itself, not from a host, so a stopped
  session shows its branch too.
- When a row is too wide beside the preview, the middle identity columns are
  clipped, keeping `Last active` and `Status` together.
- The current session is the initially selected row.

**`Last active`** uses one unit at a time: `5min ago`, `3h ago`, `5days ago`.

- Partial units round up, even across a boundary: 59 minutes and one second
  reads `1h ago`.
- The current session reads `0min ago`. Leaving it records the end of the
  visit, and ages keep advancing while the manager is open.
- A history entry with no recorded visit reads `-` until visited again.

**`Status`** reports plugin work first, in this order, and otherwise terminal
output:

| Value | Meaning |
| --- | --- |
| `CANCELLING` | Plugin activity awaiting cleanup |
| `ACTIVE` | A continuing plugin activity lease |
| `WORKING` | A finite plugin job |
| `QUIET` | The host has live terminals and none has completed a new line for two minutes |
| (empty) | No live terminals or protected plugin work, a stopped session, or a host from another protocol version |

Two conditions are also written on the row:

- `missing directory` in the status: the project directory is gone while its
  host still runs.
- `health unavailable`: a running host did not answer its health request.

- The selected row's preview names each activity's owner, title, and state.
- For `QUIET`, a new terminal starts the two-minute clock without counting its
  empty first row. Line feeds, index/new-line controls, automatic wraps, and
  top-anchored primary-screen scrolls complete lines. Partial text,
  carriage-return rewrites such as spinners, cursor-only movement,
  application scroll regions, alternate-screen repainting, and resizes do not.
  Exited terminals do not count. `QUIET` does not claim a process is idle,
  blocked, finished, or unhealthy.
- A `health unavailable` row shows `-` for terminals, buffers, unsaved
  buffers, waits, and attached TUI. They are unknown, so a missing count is
  not evidence that the session is safe to stop. A confirmed zero shows `0`, and the unsaved count is the
  host's own answer, so a healthy row showing `Unsaved 0` is one the host will
  agree to stop.
- A `missing directory` row keeps its number and place so it can be found and
  closed. Its history survives for the same reason. A stopped session with
  nothing left to open leaves the list without giving its digit away.
- While open, the manager asks each compatible running host for this bounded
  health at most once every five seconds. It never fetches terminal contents
  or reads activity from the preview.

#### Preview

`Ctrl-t` toggles the preview in the right column. The choice holds while the
selection moves and rows refresh; reopening the manager starts fresh.

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

- Every row answers the same questions in the same order, so two sessions are
  compared by reading one place.
- `-` means nothing can answer, which is deliberately not `0`.
- `Terminals 2 (1 exited)` counts retained exited screens beside live ones,
  since they are not live state.
- `Panes` comes from a bounded, read-only request for the selected row only,
  which never becomes a second attachment. It reads `…` while in flight and
  `-` for a stopped session or another protocol version.
- `Worktree` is `yes` only for a linked Git worktree; `no` for a main checkout
  or a non-repository. A `.git` file alone does not make a worktree: a
  submodule and a `--separate-git-dir` repository both have one for their own
  main checkout.
- Nothing in the preview is saved for later listings.
- Buffer and terminal contents are not shown: at this width a snippet is
  neither readable nor useful as identity.

#### Numbers and order

Sessions carry a number from `1` to `9`.

- **Order.** Numbered sessions lead in digit order, so the digit tells both
  which key reaches a row and where it is, and the top of the list stops
  rearranging as you visit. The rest follow by `Last active`, least recently
  visited first, with never-attached sessions (`-`) at the very bottom.
- **Only running sessions are numbered.** A stopped row has no digit, drops
  below the numbered ones, and gives up all numbering state. A stopped row is
  drawn in the dimmed text colour, so running hosts stand out without hiding
  any row.
- **Automatic numbers** close gaps in their existing order, and a new session
  takes the lowest free digit: with three running sessions, the next is `4`
  even if stopped history used to hold it. Sessions with no recorded creation
  order are numbered most-recently-visited first.
- **Only nine** are numbered at a time; a tenth is reached by name or path.
- **Missing numbers.** A numbered shortcut with no session reports an error.
  Shortcuts never restart a stopped session or take over an occupied TUI.

**Renumber** (Tab menu, running rows only) opens a prompt for one digit, then
returns to the manager.

- A number another session holds is swapped, so both keep a shortcut. The
  displaced session may be renumbered automatically later.
- An empty answer removes the number and keeps it removed while the session
  runs: the session stays in the by-visit part of the list until numbered by
  hand or stopped.
- A number chosen this way is pinned while the session runs and reserved
  before automatic numbering.

### Session and destination navigation

#### The Navigator

`Space n` (or `Ctrl-w n`) opens the **Navigator**: a fuzzy picker of open
buffers and running terminal sessions.

- `Ctrl-w n` works in editor modes and Terminal Insert and follows
  `keys.window`. `Space n` is child input in Terminal Insert.
- No filesystem scan or content search runs here; `Space f` is the Finder.
- With `editor.auto_close_terminal: false`, exited terminals stay in
  `Space t t`, whose Tab menu offers **Close all
  exited terminals** without touching live children.

The Navigator, the buffer list (`Space b b`), and the terminal list
(`Space t t`) are one list over three scopes — every open destination, the
buffers, or the terminals including retained exited ones — with the same columns:

```text
    TYPE        NAME                           STATE
  * [scratch]   scratch                        [+]
  * [file]      README.md                      [+]
    [about]     about                          [RO]
    [file]      /tmp/prompt-8e155cfe…90d4.md   [STALE]
    [explorer]  .
  * [terminal]  ✳ Article usage in mode descriptions
    [terminal]  user@host:~/code/runyte-dev    exited
```

| Column | Shows |
| --- | --- |
| `*` | A pane is showing it |
| TYPE | The bracketed kind a pane title uses, coloured by kind: file, explorer, generated page (`[about]`, `[config]`, `[help]`), scratch, terminal. A rendered Markdown page is `[rendered]`, with its source name in NAME. |
| NAME | A path relative to the workspace, under `~` inside your home, or absolute; or a terminal's name. Too-long names are shortened in the middle, keeping the file name. |
| STATE | Every flag that applies: `[+]`, `[STALE]`, `[RO]` for buffers; `exited`, `unread`, `bell` for terminals. A conflicting edit reads `[+] [STALE]`. |

- Scratch and retained special buffers appear once per identity.
- Lists open in recent activation order, and keep that order while filtering.
  The terminal list puts running terminals first and dims exited ones.
- Names, paths, terminal titles, launch commands, and running terminals'
  numbers (`3` or `#3`) match fuzzily, with matched characters emphasized.
- A terminal's preview shows its live screen, updating as output arrives and
  as the selection moves, even for hidden or reviewed terminals. Previewing
  does not focus or resize it. A running terminal's number is in its pane
  title (`[terminal #3] …`).

| Key | Action |
| --- | --- |
| Enter | Visit |
| Tab | Resource actions |
| `Ctrl-n` / `Ctrl-p`, `Ctrl-d` / `Ctrl-u`, Home / End | Move and page |
| `Ctrl-t` | Toggle the preview (shown at first) of buffer contents or the live terminal screen |
| Delete | Clear the filter |
| Escape, `Ctrl-c`, or the leader with an empty query | Cancel |

- Printable `j`, `k`, and `q` filter.
- Cancelling from Terminal Insert resumes child input. Visiting a document
  enters Normal; visiting a live terminal resumes Insert unless it holds a
  review.
- Visiting a destination already visible focuses its pane. **Bring into
  active pane** places it here instead. A PTY always has one live view.

**Previous destination.** `Ctrl-w p` (`previous-destination`) returns to the
previous open destination in this pane, skipping closed resources and exited
terminals.

#### Switching sessions

- **`Shift-Left` / `Shift-Right`** visit the previous/next running session in
  manager order, including unnumbered ones, wrapping at the ends. They work in
  Insert and Terminal Insert as remappable exceptions to child input, subject
  to open overlays and confirmations. They never start a stopped session or
  force an occupied attachment. If an outer tmux takes Shift-Left/Right,
  release or change those tmux bindings.
- **`Ctrl-w a`** (`previous-session`) alternates between the last two
  successful attachments. Failed switches leave that history alone.
- Every attachment refreshes the session list in the background, including
  when returning to a host whose cached list predates a newly started session.

#### The session strip

The session strip sits above the editor area.

| `workspace.session_strip` | Shown |
| --- | --- |
| `auto` (default) | With more than one running session |
| `always` | Always |
| `hidden` | Never |

- Zen hides it.
- Left-click an entry to switch to that running session, numbered or not.
  Clicking the current entry, empty space, or the overflow count does nothing.
  Prompts and overlays keep input ownership.
- Entries keep manager numbers and names. Overflow keeps the current entry
  visible and shows how many were left out as a trailing `…N`.
- Every entry has exactly one state marker, so a name sits in the same cells
  whatever its session is doing. In order of precedence:

  | Marker | Meaning |
  | --- | --- |
  | `?` | Unknown health (distinct from stopped) |
  | `!` | A terminal bell |
  | `+` | Unread terminal output |
  | `·` | Quiet |

- Unread output and bells follow the terminals' viewing rules: entering a
  session does not acknowledge its hidden terminals. `QUIET` in the manager
  still describes completed-line output, not job completion.

#### Opening another directory

- **The directory chooser** (`Ctrl-o` in the manager) offers recent roots,
  sibling worktrees, typed paths, and directory browsing. **Open this
  directory** selects the shown root; descending into a child is a separate
  action. On Windows it accepts drive paths and backslashes; recent-root and
  worktree shortcuts are Unix-only for now. Worktree creation is explicit.
- **`Tab s` in an explorer** offers **Open persistent session here** for the
  directory it shows, even when empty.
- **A terminal's action menu** can open its last validated OSC 7 directory,
  and explains when there is none.
- **Open destinations** (`Ctrl-e`) inspects the selected running session's
  open destinations; Escape returns. Choosing one attaches and revalidates it;
  a resource closed meanwhile leaves the restored layout intact and reports
  the loss.

These routes initialize or reuse the exact directory's persistent session and
keep the source host and its terminal processes.

**From inside an integrated terminal**, `cd ../worktree` then `runyte -a`
switches the outer TUI to that directory's workspace, a Git worktree here.
Outside any workspace it refuses; `runyte -a DIRECTORY` names a directory to
make one.

- Relative arguments resolve from the shell's directory.
- The command returns to the shell without starting a nested TUI; switching
  back returns to that shell.
- The route checks the owning host, terminal, and caller, and reports the
  attachment or its error. Stale, detached, or ide-mode parent contexts get
  an actionable error.
- Outside integrated terminals, launching works as usual.

### Managing sessions from the command line

```sh
runyte --mux [WORKSPACE]                # or runyte -a [WORKSPACE]
runyte --session-stop [WORKSPACE]       # or runyte -s [WORKSPACE]
runyte --session-restart [WORKSPACE]
runyte --session-rename WORKSPACE NAME
runyte --session-list                   # or runyte -l
runyte --session-list --include-hidden
runyte --session-stop-all
runyte --session-stop-all --include-hidden
runyte --session-clean
```

Omitting `WORKSPACE` from attach, stop, or restart selects the project found
from the current directory.

#### Selecting a session

`WORKSPACE` may be:

- the abbreviated ID a listing shows, any other unambiguous ID prefix, or the
  full ID;
- the exact session name;
- the project directory.

**IDs** are stable hashes of canonical project directories, shown abbreviated
to six characters. A listing lengthens them only when two of its own rows would
otherwise match, so the ID shown always selects that row.

**Names:**

- A project is named after its directory when first recorded. A taken name
  gets the first free suffix from `-2`: three `runyte` directories become
  `runyte`, `runyte-2`, and `runyte-3`. Unnamed history entries get defaults on
  the next listing.
- `runyte --session-rename WORKSPACE NAME` renames a running session (through
  its host) or a stopped one (in its history).
- Names persist across restarts and must be unique among running sessions.
- Surrounding spaces are trimmed and inner spaces become `-`:
  `  release candidate  ` is stored as `release-candidate`. Default names
  follow the same rule.
- Names live under the workspace state root, normally `.runyte/host-names/`.

#### Listing sessions

`runyte --session-list` shows running and recently visited sessions with `ID`,
`NAME`, `DIRECTORY`, `STATE`, `UNSAVED`, `TERMINALS`, `WAITING`, `JOBS`,
`ACTIVITIES`, and `TUI` columns.

- Order matches the manager: numbered sessions in digit order, then the rest
  least recently visited first. With no number column, this reads as running
  sessions ahead of stopped ones.
- `STATE` is `running`, `stopped`, or `running (protocol N)` for a host from
  another Runyte version. Such a host still holds the workspace, so nothing
  can attach or open files through it, and its unsaved counts are unknown.
- It reads the Runyte environment selected by the current runtime and cache
  settings. `--include-hidden` adds validated live sessions from other
  isolated Runyte environments. Stopped history stays local, since there is no
  live host to publish it. If two environments host the same workspace, both
  endpoints appear with the same ID and directory.

**History and names** live in `workspaces.json` in Runyte's platform cache
directory: `$XDG_CACHE_HOME/runyte` when set, else `~/.cache/runyte` on Linux
or `~/Library/Caches/runyte` on macOS. Without the XDG override, Unix takes the
home directory from the effective operating-system account, not `$HOME`, so a
privileged launch cannot create its cache as root in another account's home.
Because this is per-user cache rather than the runtime registry, stopped
projects stay listed across logout.

#### Stopping and restarting

**Protected state.** Stop and restart refuse while the host owns:

- unsaved buffers,
- pending `--wait` requests,
- live terminal children,
- active plugin jobs or continuing activity leases.

The refusal names each count. Add `--force` to discard that state.

- The scratch buffer never counts as unsaved: it has no path, so nothing could
  be saved in place. A scratchpad never keeps a workspace alive, and stopping
  or retiring the host discards it.
- A normal stop never kills a host from another protocol version, since it may
  own live terminals or unsaved buffers. Use a compatible client, or make the
  loss explicit with `runyte --session-stop --force`.

**Restart** replaces the running host without attaching a TUI, and keeps its
name. It does not keep clean buffers or other in-memory state. If the host
used a non-default configuration, pass the same `--config PATH`.

**`--session-stop-all`** applies the same checks to every running session in
the current environment, and continues past refusals so unrelated clean hosts
still stop. `--include-hidden` applies it to every validated live endpoint in
the owner-wide inventory, including copies of one workspace in several
environments. `--force` makes the loss explicit for every host.

**`--session-clean`** forgets every stopped row from history after rechecking
the inventory. Running sessions, workspace directories, and project files are
untouched.

#### Idle hosts

By default a persistent host keeps running after detach, with its language
servers, until stopped with `:session-stop` or otherwise shut down. Set
`workspace.idle_retirement_minutes` to a positive number to retire a clean host
after that many minutes without an attached client, an outstanding `--wait`
request, or a live terminal child. The default `0` never retires.

### External editor requests (`--wait`)

For tools that need an editor process to stay open until you finish, use
`runyte --wait`:

```sh
git config core.editor 'runyte --wait'
```

That gives Git commit and rebase message files this lifecycle. On Unix,
persistent hosting and `--wait` use a private, versioned local protocol: a
bundled-client contract, not a public automation API.

#### On Unix

`runyte --wait file…` opens the files in a persistent session and returns
success only after every requested buffer is closed or completed.

**Where the file appears:**

- It reuses a matching buffer in an existing host. A clean buffer left by an
  earlier completed wait is refreshed from disk first. Unsaved text and
  buffers owned by a pending wait are never replaced.
- If the host already has a TUI, the file appears there while the caller
  waits. If that TUI detaches first, the calling terminal takes over.
- If no host exists, Runyte starts one and attaches the calling terminal, so
  the request is never invisible.
- The buffer opens in Normal mode, even if the session was in Insert mode, so
  `:` commands work at once.
- If the pane was showing an integrated terminal — usually the one running the
  program that asked — the terminal is covered, not given up. Finishing with
  `:q`, `:q!`, `:wq`, `:close`, or `:wbc` shows that terminal again in the same
  pane. Navigating the pane to another document first ends the detour, after
  which `:q` closes the pane as usual.

**Finishing:**

| Command | Result |
| --- | --- |
| `:wbc` | Write and close the requested buffer without changing the layout; the host and other buffers keep running |
| `:wq` | Write, then `:q` — which from the last pane stops a clean persistent session |
| `:q` on a clean buffer | Complete without writing |

- Dirty buffers keep the usual save/discard protection.
- Detaching the wait-owned TUI cancels the request.
- Losing the calling terminal or process cancels it, whether the client is
  queued behind another TUI or has taken over.
- Lifecycle loss, explicit cancellation, and host failure all exit nonzero.

#### On Windows

From an ordinary shell, `runyte --wait` opens the files in the current
project's persistent session, starting it if needed.

- With no attached TUI, the calling terminal attaches. With another TUI
  active, the caller waits for its buffers there and can take over after that
  TUI detaches.
- Each requested buffer must complete before the caller resumes. `:wbc` saves
  and closes one buffer; `:q` can finish a clean wait.
- Discard, detach, terminal loss, or loss of the launching process cancels it.
- The session is held by its exact publication throughout, so a second live
  publication for the same path is refused as ambiguous.

#### From an integrated terminal

Programs in an integrated terminal can use `EDITOR='runyte --wait'` and
`VISUAL='runyte --wait'`. Their files open as ordinary buffers in the parent
persistent session, even from another working directory or a temporary path.

- Relative files resolve from the calling terminal's directory.
- The origin terminal is covered while the edit is active and restored after.
- The caller resumes only after every requested file completes; closing one
  of several keeps it waiting.
- The parent host authenticates the exact terminal process and attachment. A
  copied, stale, or ide-mode parent marker is refused.
- Completion and refusal never submit Enter or approve an editor prompt.

| Command | Result |
| --- | --- |
| `:wq`, `:wbc`, or `:w` then `:q` | Save and finish, keeping the pane and live terminal |
| `:w` alone | Save; the request stays pending |
| `:q` on a clean buffer | Finish without writing |
| `:q` on a dirty buffer | Refused, protecting the edits |
| `:q!` | Cancel with a nonzero result, discarding unsaved changes unless another pending request shares them. An earlier explicit save stays on disk. |

- A failed save leaves the edit open.
- Navigation and splits keep request ownership; unrelated buffers keep
  ordinary quit behavior.
- On Unix, switching sessions and explicit detach keep the request pending in its host,
  including closing a persistent-session window. Reattaching either frontend
  resumes the edit. Loss of the calling process cancels it; buffers are kept
  and no nested TUI takes over the terminal. Outside-shell control-client waits
  retain their cancellation on disconnect.

**Environment in new terminals.** New persistent terminals add `--wait` to
inherited `EDITOR` and `VISUAL` values that name bare `runyte` (including a
quoted executable path). Other editors, and commands with explicit arguments,
keep their setting. Existing shells and programs keep their old environment:
set both variables to `runyte --wait` before restarting such a program, or open
a new terminal. Ordinary `runyte <file>` commands in a shell launch as usual.

### Runtime files and discovery

This section describes where hosts publish themselves. Most people never need
it.

**Supervising `--serve`.** `runyte --serve` runs the host in the foreground,
for direct supervision or diagnostics. Runyte also uses it internally when
attaching to a missing session or restarting one.

- Once startup begins, a foreground `--serve` watches the process that
  launched it and exits when that process exits.
- Linux uses a stable process descriptor when the kernel provides one, and
  macOS a process event queue, so PID reuse cannot transfer that ownership. If
  the kernel denies stable observation, Runyte falls back to checking the PID;
  Linux also detects an unreaped zombie then.
- Unix cannot identify an original parent that vanished before Runyte started
  executing. A service manager launching `--serve` should therefore keep and
  stop the Runyte process directly.
- Hosts detached internally by Runyte are independent of the launcher and keep
  serving after it returns.
- Detached startup carries the already-resolved workspace identity, so the
  child never rediscovers a project its parent resolved.

**Endpoints.**

- Endpoint metadata and the Unix-domain socket prefer a valid owner-only
  `$XDG_RUNTIME_DIR`, falling back to the workspace state root, with
  owner-only permissions.
- A private user-wide cache registry makes both locations listable.
  XDG-backed hosts also publish a runtime copy, so a missing or unusable cache
  does not prevent discovery.
- Dead registrations are removed while listing, and stale sockets are
  recovered when a new host starts.
- Graceful retirement also removes the host's empty private endpoint
  directory.

**The owner-wide inventory** exists so an explicit `--include-hidden` can find
hosts started in other isolated Runyte environments.

- Each host publishes an owner-private row below non-disposable account
  state: `~/.local/state/runyte/all-hosts/<boot>` on Linux,
  `~/Library/Application Support/Runyte/all-hosts/<boot>` on macOS. The home
  directory comes from the operating-system account database, not `$HOME`.
- The final directory is keyed by the kernel's boot identity, so machines
  sharing a home never overwrite or inspect each other's PIDs and sockets, and
  a new boot inherits no stale process identity. This stable, account-owned
  parent cannot be pre-claimed by another user in the system temporary
  directory.
- If the account home or boot identity cannot be resolved, publishing a new
  host and explicit hidden-session operations fail with an error rather than
  using an incomplete inventory.
- Ordinary discovery, identity locking, attaching to a running host, and
  ordinary listing never read or depend on it. Broad listing reads the current
  environment's history for display but never rewrites it from hosts found
  elsewhere.
- Scans accept only private, non-symlinked records whose workspace identity,
  endpoint metadata, live process, and responsive socket agree where process
  visibility allows. A responsive endpoint is kept when a PID namespace hides
  its process; one that cannot be observed conclusively is left out without
  being removed.

### Change the shell directory on exit

Like Yazi, Runyte cannot change the directory of the shell that launched it.
Start it through a shell function that passes `--cwd-file`; then `:quit-here`
(`:qh`) exits and moves the shell.

| Command | Shell directory afterwards |
| --- | --- |
| `:quit-here` / `:qh` | The active explorer directory, or the active file's parent |
| `:quit-here!` / `:qh!` | The same, discarding unsaved changes |
| `:quit` from the last pane, `:quit-all` | Unchanged; returns to another running session when possible |

A pathless view uses the last explorer directory visited in that pane, then
the working directory set by `:cd`. `:quit-here` keeps the normal
unsaved-change protection.

#### Bash or Zsh

Add this to the shell configuration:

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

#### Windows PowerShell 5.1

Save [runyte.ps1](../../contrib/runyte.ps1) somewhere stable and dot-source it
from your PowerShell profile:

```powershell
. 'C:\Tools\Runyte\runyte.ps1'
```

- The function starts `runyte.exe` and waits for it. `:quit` leaves the
  caller's directory alone; a successful `:quit-here` changes it using a
  literal path.
- It returns the editor's status in `$LASTEXITCODE`, or a nonzero status if
  the wrapper fails.
- The handoff needs private local NTFS temporary storage.
- The destination needs an identity-equivalent ordinary local or UNC spelling
  shorter than 260 UTF-16 units, with valid Unicode components. Other paths
  are refused before quitting.
- The bounded, versioned handoff record keeps UTF-16 code units; it differs
  from the Unix NUL-terminated byte format.
- PowerShell 7 has not been validated for this wrapper.

#### How it behaves

- After reloading the shell configuration, run `runyte` as usual.
- `--cwd-file` is for shell integration. Runyte writes it only after a
  successful `:quit-here`.
- Without the wrapper, `:quit-here` refuses to exit and explains how to enable
  it, rather than acting like `:quit`.
- Session commands such as `--session-list` accept the option but leave the
  file alone, so they work through the same function.
- **Persistent sessions** (Unix and Windows): `:quit-here` runs in the host,
  which reports the chosen directory, and the attached client writes the
  file. The same wrapper serves both modes, and the directory follows you
  across workspace switches.
- The capability belongs to the client, not the host: a client launched
  without the wrapper is refused even if an earlier one had it.
- On Windows, the frontend writes the handoff after restoring the terminal.
- In mux mode `:quit-here` stops the session after the same checks as
  `:quit`. Use `:detach` to keep the host running.

<!-- navigation -->
[Contents](README.md) · [Previous: Language support](language-support.md) · [Next: Commands](commands.md)
