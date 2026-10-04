---
title: "Editing files and directories outside a workspace requires adopting one"
status: resolved
reported: 2026-10-04
resolved: 2026-10-04
commit: 0c2fcd7
---

## Resolution

Fixed in `0c2fcd7` ("Open files and directories outside a workspace as plain
sessions").

Launch resolution in `src/main.rs` asked one question of every launch — which
workspace owns it — and when discovery found neither a Git root nor a state
directory, `project_root::prompt` stopped startup to ask, whatever the launch
named. `launch_is_plain` now decides first. A standalone launch with a file or
directory target goes plain when discovery finds nothing, `--plain` forces it,
and a target launched as root is always plain; `--init`, `--project-root`, a
persistent attachment and a bare launch are never plain, and discovery is only
run when its answer matters. `refuse_wait_as_root` refuses `--wait` as root,
because it opens files through a persistent session and would leave a
root-owned host in whatever workspace the launch directory belongs to.

A plain launch builds the editor with `App::new_plain_with_deferred_syntax`,
which neither validates nor creates the state directory, and skips the
recent-workspace record and the default log (`--log PATH` still installs
one). `start_host_services` checks `is_plain()` and leaves the context
receiver closed and Git, the session catalog and plugins absent. It still
spawns the language-server manager so the loop has its channel, but never
configures LSP permission or attaches the manager, so no document is offered
and no server starts. The Windows native catalog is skipped the same way.

Deviation from the constraint that `project_root` become optional: it stays a
required path. A plain session keeps the launch directory there only as the
anchor that relative paths are displayed against, and a `plain` flag on `App`
(`src/app/plain_session.rs`) is what every project-scoped feature consults. The
editor layer reads `project_root` in many places that are display-only, while
the persistent-host and catalog code never sees a plain session; an explicit
flag made each workspace-scoped decision visible without rewriting those
readers. The features that would otherwise have fallen back to the launch
directory each take the plain path explicitly: `search_root`, the scan scopes,
`literal_navigation_candidates` and insert-mode path completion for `g f`, and
`open_terminal`.

Refusal goes through the existing capability snapshot rather than per-command
guards. `command_capabilities` replaces every Git, LSP and session capability
with `PLAIN_SESSION_REASON` (`needs a workspace; use :workspace-init`) in a
plain session. Two capabilities were added: `Workspace` for the commands that
had none (`:mcp`, `:lsp-trust`, `:plugins`, `:plugin-stop`,
`:plugin-restart`) and `PlainSession` for `:workspace-init`.
`plain_session_refusal` derives from the same capabilities, and
`execute_selection_action`, `execute_editor_command_action` and the colon
dispatcher refuse through it, so key bindings, the palette and protocol
callers cannot disagree with what help and key hints grey out. Pasting an image
is refused too, because pasted images are stored in the state directory.

Project-wide search is redirected rather than disabled. `open_project_picker`,
`open_all_files_picker`, `open_project_grep` and `open_global_search` use
`search_root`, which is the active file's or explorer's directory in a plain
session. Their scans use the new `ScanScope::Contained`, bounded by
`src/scan_boundary.rs`: `refusal` rejects the filesystem root and virtual
filesystems (the `/proc`, `/sys` and `/dev` mount points by name, since
devtmpfs reports the tmpfs magic number, plus kernel pseudo-filesystems by
`statfs` magic on Linux); `Boundary` keeps the walk on the root's device; and
both walkers stop after reading `CONTAINED_SCAN_ENTRY_LIMIT` (20,000)
directory entries and report the result as limited. The budget is spent while
entries are read from the directory, before collecting, sorting or filtering,
so hidden and ignored entries count and a single huge directory is never
collected whole; a first version counted only admitted entries after reading
each directory in full. `scan_with` in `src/file_picker.rs` returns a
`ScanTally` so the cap reaches the picker as `limited`, and the contained
workspace-search walk passes over unreadable subdirectories instead of failing,
since it starts in system directories the reader only partly owns. Instead of a
literal `Finder: <dir>` title, the existing scope label always names the root
for a contained scope (`<dir>` or `all files in <dir>`). The search prompt
reads `search <dir>: `, and the results page gains a `Directory:` line.

`:workspace-init [directory]` calls `project_root::initialize`, as `--init`
does, against the same per-user roots the launch was validated with: startup
hands the editor its list through `note_reserved_user_roots`, so a `--config`
file outside the default configuration directory is protected with its
launch-relative `config_root_for` resolution rather than the default
directory. It then `adopt_workspace` moves `project_root`, `state_root` and the working
directory to the new root and sets a request the standalone event loop reads at
the top of each turn. `start_workspace_services` refreshes the host's
`WorkspaceIdentity` first — context registration pairs it with the project root
and is rejected when they differ — then starts the context endpoint, Git, a
language-server manager rooted at the new workspace, the session catalog and
plugins, and records the workspace. On Windows the session catalog is the
native one, so `standalone_native_catalog_config` and `spawn_native_catalog`,
shared with launch, start and attach it and its owner and receiver join
`HostServices`.

The status row reads `plain │ Directory:` through a third `SessionMode` in
`src/ui.rs`, and filesystem-plan titles say `as root` when `running_as_root` is
set. Readiness was measured with `benchmarks/startup.py`'s `measure` and
recorded in `context/reference/startup-performance.md`: it is within
run-to-run spread, because every omitted service already started after the
first frame.

Tests:

- `src/app/tests/plain_session.rs`: capabilities, refusals by typed command and
  key binding, Finder and search scope, the search prompt and results header,
  `g f`, `:workspace-init` with and without a directory, on an unusable one, and
  against a state path inside the loaded configuration directory,
  host identity after init, greying of workspace-only and plain-only commands,
  and the `as root` plan title.
- `src/main.rs`: `a_launch_is_plain_only_with_targets_and_no_workspace_or_when_asked`,
  `wait_is_refused_as_root_and_nowhere_else`,
  `plain_keeps_a_bare_launch_off_the_persistent_default`, and
  `a_plain_host_starts_no_workspace_service`.
- `tests/plain_session.rs`: real launches showing that a file launch outside a
  workspace neither asks nor writes state, a bare launch still asks, `--plain`
  keeps a launch inside a workspace out of its state directory, and `--log`
  still writes.
- `src/launch.rs`: `plain_is_a_standalone_option_that_opens_no_workspace`.
- `src/scan_boundary.rs` tests, the `a_contained_*` tests in
  `src/file_picker.rs` and `src/workspace_search/tests/mod.rs` (including the
  budget spent on hidden and ignored entries), and
  `a_plain_session_names_itself_in_the_rendered_status_row` and
  `the_status_row_names_the_workspace_mode_before_the_workspace` in
  `src/ui.rs`.

Known limitation: `:workspace-init` does not move diagnostic logging into the
new state directory, because a logger is installed once per process; a plain
session that should keep a log is started with `--log PATH`. It also leaves the
process working directory alone, as `:cd` does. `start_workspace_services` has
no direct test, because starting the context endpoint writes to per-user
context storage; the identity test covers the registration mismatch it guards
against. Off Unix, a contained scan has no filesystem boundary or
virtual-filesystem detection, only the filesystem-root refusal and the entry
cap.

## Report

Every launch resolves a workspace before the editor opens. Discovery walks up
from the launch directory looking for a Git root, then for the configured
state directory (`.runyte/`). When neither exists, startup stops and asks:

```
No Git repository or existing project workspace directory was found.
Project directory [/etc]:
```

This happens even when the launch names only a file. It makes Runyte awkward
as a general-purpose editor for administrative work and quick config edits:

- `sudoedit /etc/fstab` with `SUDO_EDITOR` pointing at Runyte, run from `/`,
  `/etc` or the home directory, asks the question before showing the
  temporary copy (for example `/var/tmp/fstabXXXX.fstab`). Accepting `/etc`
  then fails, because the invoking user cannot create `/etc/.runyte`.
- `sudo runyte /etc/fstab` run from `/etc` creates a root-owned `/etc/.runyte/`
  if the question is accepted. Run from inside a user's Git checkout, it adopts
  that checkout as the workspace and writes its standalone log, and possibly
  `.runyte/` itself, as root inside the user's repository.
- Even when no question is asked, a file launch starts every service that
  assumes a project root: Git discovery and its monitor, the language-server
  manager, plugins, the agent/MCP context endpoint, the file monitor, the word
  index, workspace search and the pipe service (`start_host_services` in
  `src/main.rs`). It also writes `.runyte/standalone-<pid>.log` and a
  recent-workspace entry.
- `runyte DIR` outside any workspace asks the same question, so the explorer
  cannot be used as a file manager for system directories without first
  turning them into workspaces.

### Expected behavior

A standalone session may have no workspace. This is called a plain session.
It is not a third launch mode: standalone versus persistent says where editor
state lives, and having a workspace or not says whether a project root exists
to scope project-wide features. A persistent session is the live state of one
workspace, so a plain session is always standalone.

"Workspace found" below means a Git root or a configured state directory in
the launch directory or one of its ancestors, as discovery does today. A Git
checkout remains a workspace without `.runyte/`.

| Launch | Workspace found | No workspace found |
|---|---|---|
| `runyte FILE...` | Standalone in that workspace (unchanged) | Plain, file open (today: asks) |
| `runyte DIR` / `runyte .` | Standalone in that workspace, explorer (unchanged) | Plain, explorer (today: asks) |
| `runyte` (bare) | Follows `workspace.mode` (unchanged) | Asks (unchanged) |
| `runyte -a` | Persistent (unchanged) | Creates `.runyte/` in the launch directory, persistent (unchanged) |
| `runyte --init DIR` | Standalone at exactly `DIR` (unchanged) | Same (unchanged) |
| `runyte --plain FILE...` / `runyte --plain DIR` | Plain | Plain |
| Effective user is root, with a file or directory target | Plain unless `--init` or `-a` is given | Plain |

`--wait` opens its files through a persistent session, so as root it is
refused rather than made plain.

`--plain` lets a quick edit stay plain from inside a repository, for example
`SUDO_EDITOR="runyte --plain"`; sudo splits that value into arguments. The
root rule keeps `sudo runyte` from writing root-owned runtime state into a
user's workspace. `sudoedit` remains the recommended way to edit single system
files, because the editor then never runs with root privileges.

#### Status row

The status row names the mode in the place that already reads `standalone`
or `persistent`: a plain session reads `plain`. The directory after it is
labelled `Directory:` rather than `Workspace:`, because there is no
workspace.

```
 NOR │ plain │ Directory: /etc
```

#### Kept in a plain session

Editing, the keymap, user configuration and theme, syntax highlighting, undo,
registers and clipboard, atomic saves, splits, help, `:open` with path
completion, integrated terminals (started in the active buffer's or explorer's
directory), and the editable explorer with confirmed filesystem plans.
Deletions go to the system trash as they do today.

#### Absent from a plain session

Workspace discovery and the startup question, the state directory, the
default log file (`--log PATH` still writes one), the recent-workspace entry,
persistent sessions, the agent/MCP context endpoint and plugins.

Git, language servers and MCP are disabled completely, not just left idle:

- No Git process runs at any point in a plain session: no repository
  discovery, no Git monitor, no gutter line marks, and no Git views or
  commands, even when an open file lies inside a Git checkout.
- No language server is started, and there is no command to start one on
  demand. Diagnostics, completion from servers, hover, go-to and the
  symbol, reference, diagnostic and code-action pickers are unavailable.
- No agent context endpoint is published, so `runyte mcp` cannot reach a
  plain session and `runyte --context-list` does not list one. No context
  grant is read or written, and `:mcp` is unavailable. Terminals opened in a
  plain session carry no context credentials, so an agent running inside one
  cannot reach the editor either.
- Git, language-server and MCP commands invoked in a plain session report
  that they need a workspace rather than failing silently, and key hints and
  help mark them unavailable through the existing command-availability path.

#### Project-wide search becomes directory-scoped

There is no project root, so:

- `Space f` opens the Finder over the active buffer's directory, or the
  explorer's directory, as `Tab f` and `:file-picker-directory` do today. Open
  buffers and terminals are still ranked into it. Editing
  `/etc/nginx/nginx.conf`, it finds files under `/etc/nginx/`; in an explorer
  at `/etc`, it finds files under `/etc`.
- The project search under `Space /` (`Space / s`, `Space / /`) searches below
  the same directory, as `:fuzzy-grep-directory` does.
- The Finder and search prompts show the directory they cover, for example
  `Finder: /etc/nginx`, because the scope follows the active directory rather
  than a fixed root. Prompts in workspace sessions are unchanged.

The launch directory is deliberately not the scope: it is often `/` or the
home directory and unrelated to the file being edited.

#### Recursive scans need a guard

Directory-scoped scans in plain sessions start from arbitrary system
directories, so they must not walk the whole machine:

- stay on the filesystem of the scan root;
- skip virtual filesystems such as `/proc`, `/sys` and `/dev`;
- use an entry cap stricter than project scans;
- refuse a recursive scan rooted at `/`, with a message suggesting a narrower
  directory.

#### Adding a workspace later

`:workspace-init` turns a plain session into a workspace rooted at the active
explorer's directory, or the active file's directory. Like `--init`, it creates
the state directory there (or uses an existing one) and keeps running
standalone; `runyte -a` from that directory later attaches persistently.
Once the session has a workspace, the services a plain session leaves off
(Git, language servers, plugins, the context endpoint) start as they would
for a standalone launch in that workspace.
Without it, opening `runyte .` in a new non-Git project would be a dead end
under the new rule.

#### Running as root

When the effective user is root, the filesystem-plan confirmation says so
explicitly. The user guide notes that root's deletions go to root's trash or
the mount's trash directory, not the invoking user's.

#### Documented sudo flow

The user guide gets a short section on editing system files that recommends
this flow:

1. Point `SUDO_EDITOR` at an installed binary in the shell profile, for
   example `export SUDO_EDITOR="runyte --plain"` with `runyte` on `PATH`. A
   path into a build directory such as `target/release/runyte` breaks after a
   clean or during a rebuild.
2. Edit single files with `sudoedit /etc/fstab`. sudo copies the file to a
   temporary path, runs Runyte as the invoking user with that user's
   configuration and theme, and copies the result back as root after the
   editor exits. Quitting without saving leaves the file unchanged. Runyte
   itself never runs as root.
3. Use `sudo runyte /etc` (plain by the root rule) only for operations
   `sudoedit` cannot do, such as renaming, moving or deleting files in system
   directories through the explorer. The editor then runs as root, and
   deletions go to root's trash. Configuration is found through
   `XDG_CONFIG_HOME` and `HOME` (`src/config/paths.rs`), so whose
   configuration applies depends on the sudoers environment policy; with the
   common defaults (`env_reset` and a `HOME` set to the target user) it is
   root's.

### Constraints

- `project_root` is currently a required path on the editor state and is read
  in roughly 13 files under `src/app`. A workspace becomes optional there, and
  every project-scoped feature needs an explicit no-workspace path rather than
  falling back to the launch directory.
- Key dispatch, help and key hints keep reading from the keymap registry; any
  binding whose meaning changes in a plain session must say so in help.
- Startup cost of plain sessions should be measured with `benchmarks/` and
  recorded in `context/reference/startup-performance.md`.
- `README.md`, the CLI help (`print_help` in `src/main.rs`) and the
  "Where workspace state lives" section of `docs/user-guide.md` describe the
  new launch table, and the user guide carries the sudo flow above.

### Reproduction

1. Set `SUDO_EDITOR` to the `runyte` binary.
2. `cd /` (or any directory outside a Git checkout without `.runyte/`).
3. Run `sudoedit /etc/fstab`.
4. Runyte asks for a project directory before the file is shown.

The same happens with `runyte /etc/hosts` or `runyte /etc` from that directory.
