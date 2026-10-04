# Editing files and directories outside a workspace requires adopting one

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

## Expected behavior

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

`--plain` lets a quick edit stay plain from inside a repository, for example
`SUDO_EDITOR="runyte --plain"`; sudo splits that value into arguments. The
root rule keeps `sudo runyte` from writing root-owned runtime state into a
user's workspace. `sudoedit` remains the recommended way to edit single system
files, because the editor then never runs with root privileges.

### Kept in a plain session

Editing, the keymap, user configuration and theme, syntax highlighting, undo,
registers and clipboard, atomic saves, splits, help, `:open` with path
completion, integrated terminals (started in the active buffer's or explorer's
directory), and the editable explorer with confirmed filesystem plans.
Deletions go to the system trash as they do today.

### Absent from a plain session

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

### Project-wide search becomes directory-scoped

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

### Recursive scans need a guard

Directory-scoped scans in plain sessions start from arbitrary system
directories, so they must not walk the whole machine:

- stay on the filesystem of the scan root;
- skip virtual filesystems such as `/proc`, `/sys` and `/dev`;
- use an entry cap stricter than project scans;
- refuse a recursive scan rooted at `/`, with a message suggesting a narrower
  directory.

### Adding a workspace later

`:workspace-init` turns a plain session into a workspace rooted at the active
explorer's directory, or the active file's directory. Like `--init`, it creates
the state directory there (or uses an existing one) and keeps running
standalone; `runyte -a` from that directory later attaches persistently.
Once the session has a workspace, the services a plain session leaves off
(Git, language servers, plugins, the context endpoint) start as they would
for a standalone launch in that workspace.
Without it, opening `runyte .` in a new non-Git project would be a dead end
under the new rule.

### Running as root

When the effective user is root, the filesystem-plan confirmation says so
explicitly. The user guide notes that root's deletions go to root's trash or
the mount's trash directory, not the invoking user's.

### Documented sudo flow

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

## Constraints

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

## Reproduction

1. Set `SUDO_EDITOR` to the `runyte` binary.
2. `cd /` (or any directory outside a Git checkout without `.runyte/`).
3. Run `sudoedit /etc/fstab`.
4. Runyte asks for a project directory before the file is shown.

The same happens with `runyte /etc/hosts` or `runyte /etc` from that directory.
