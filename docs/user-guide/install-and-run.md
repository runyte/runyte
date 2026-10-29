## Install and run

Runyte has terminal and desktop editions with the same editor, configuration,
plugins and persistent sessions. The desktop edition adds the window and media
viewing, and also runs in a terminal. Both distributions install as `runyte`;
`--window` requires the desktop edition. Clients and hosts in a persistent
session must use the same Runyte version.

Start with the [README quick start](../../README.md#installation), or use the
separate [installation guide](../../INSTALLATION.md) for edition selection,
dependencies, platform-specific steps, shell setup and updates.

### Install and update with curl

The [install script instructions](../../INSTALLATION.md#install-script-linux-and-macos)
cover the terminal edition, checksum verification, version selection and custom
install directories. The script preserves existing desktop installations.

### Manual download and source installation

See [release archives](../../INSTALLATION.md#release-archives) for terminal downloads,
[Cargo installation](../../INSTALLATION.md#install-with-cargo) for crates.io, and
[desktop installation](../../INSTALLATION.md#desktop-edition) for the window edition.

### Building from a clone

See [building from source](../../INSTALLATION.md#build-from-source) for prerequisites,
package names, build commands and installation of the resulting executable.

### Windows support

Runyte runs natively on x86-64 Windows 11 24H2 or later, inside Windows
Terminal. The rest of this guide applies to Windows too; this section lists
what is missing, what is limited, and what works differently.

See [Windows installation](../../INSTALLATION.md#windows) for release ZIPs,
checksums, the Visual C++ runtime and source-build prerequisites.

#### Not available on Windows

- The recent-root and worktree shortcuts in the session manager's `Ctrl-o`
  directory chooser. The chooser itself accepts drive paths and backslashes.
- `:session-stop` and `--session-stop` without a workspace. Name the workspace
  by ID, name, or path.
- Git and language servers installed as `.cmd`, `.bat`, or `.ps1` wrappers.
  Only native `.exe` and `.com` executables are used. To run a script-based
  language server, configure its interpreter as the executable and pass the
  script in `args`.
- Working directories whose ordinary Windows spelling is 260 UTF-16 units or
  longer. The workspace, terminals, Git, language servers, `:pipe`, and the
  `:quit-here` destination all refuse them. Opening files at such paths is not
  affected.
- UNC working directories for `cmd.exe` terminals. Use a shell that supports
  them.
- Language-server file URIs for network shares, device paths, and alternate
  data streams.
- Private runtime storage (logs, language-server approvals, the MCP adapter,
  the `:quit-here` handoff) anywhere other than local NTFS.
- Preserving unsaved editor state when the outer console window is closed.
  `Ctrl+C` and `Ctrl+Break` in the outer console request an orderly shutdown,
  but Windows may end the process before cleanup finishes after a close.
- Starting a detached persistent host when the launching process runs in a job
  that forbids breakaway. `-a` and `--session-restart` report the refusal and
  leave no new host running, and a restart checks this before stopping the old
  host. Attaching to a host that is already running still works.

#### Not validated

- ARM64 Windows
- MinGW builds
- Windows versions older than 11 24H2
- Outer terminals other than Windows Terminal
- Network shares and long-path filesystem operations
- PowerShell 7 for the `:quit-here` wrapper
- Keyboard layouts or IMEs beyond the automated input cases

When one of these fails, recoverable edits are preserved.

#### Differences from Linux and macOS

- **Configuration** defaults to `%APPDATA%\runyte\config.yaml`. A nonempty
  `XDG_CONFIG_HOME` takes precedence, and `--config` selects an explicit file.
- **Terminals** use ConPTY and start `%COMSPEC%`, falling back to `cmd.exe`.
  Command lines use native Windows quoting; see
  [Windows terminals](terminals.md#windows-terminals).
- **`:pipe`** runs Windows PowerShell rather than `/bin/sh`; see
  [Shell pipes](files-and-buffers.md#shell-pipes).
- **`:quit-here`** needs the PowerShell 5.1 wrapper
  [runyte.ps1](../../contrib/runyte.ps1); see
  [change the shell directory on exit](workspaces-and-persistent-sessions.md#change-the-shell-directory-on-exit).
- **Git** is enabled when `git.exe` or `git.com` is found on an absolute `PATH`
  entry accepted by `PATHEXT`. Without Git, Git commands are disabled with a
  reason and editing still works. Restart Runyte after installing Git or
  changing `PATH`.
- **Paste:** Windows Terminal reserves `Ctrl-v` for its own paste action.
  Runyte recognizes the empty paste event some versions send for an image,
  even while a multi-key command waits for its next key; if the terminal sends
  nothing, use `Alt-v`. Bracketed paste arrives as text in every mode, so
  pasted lines never run editor commands.
- **Saves** replace existing files with `ReplaceFileW`, which keeps their
  metadata. Another program opening the file during a save can briefly get a
  missing-file or sharing error. The `wrote` message means the whole document
  is on disk.
- **Two hosts for one project** can run at the same time. The session
  manager and the session strip show them as separate rows, and selecting one
  by path or ID is refused as ambiguous; use its name or pick the row.
- **`:session-clean`** is available inside the editor. On Linux and macOS use
  `runyte --session-clean`.
- **A bare `runyte`** with `mode: mux` attaches only to a project it can
  discover, and otherwise refuses without creating a workspace, as a bare `-a`
  does. `-a DIRECTORY` makes that directory a workspace; `--init` only creates
  one.
- **`--serve`** stops the host when the process that launched it exits.
- **Plugins** can request the `processes` capability for managed native helper
  processes.
- **`workspace.state_anchor`** chooses where retained state lives; see
  [Windows state anchor](configuration.md#windows-state-anchor).
- **External programs** for binary files open differently; see
  [Opening binary files on Windows](files-and-buffers.md#opening-binary-files-on-windows).

### Starting Runyte

| Command | What opens |
| --- | --- |
| `runyte` | The read-only about page |
| `runyte .` | The current directory in the explorer |
| `runyte file.txt` | That file |
| `runyte a.md src/main.rs` | Both files; the first is active |
| `runyte +12:4 notes.md` | `notes.md` with the caret on line 12, column 4 |
| `runyte -- +draft.md -notes.md` | Files whose names start with `+` or `-` |
| `runyte --editor /etc/fstab`, `runed /etc/fstab` | That file in [editor mode](workspaces-and-persistent-sessions.md#editor-mode), with no workspace |
| `runyte --help` | Command-line options |

Details:

- **Workspaces.** `runyte FILE` opens the file in the workspace the current
  directory belongs to. Outside any workspace the current directory becomes
  one when the file is inside it. In your home directory, a temporary or
  shared directory, or the filesystem root, or for a file elsewhere, it
  refuses and suggests `runyte --init DIRECTORY`; use `runed FILE` to edit
  without one. See [Finding the workspace](workspaces-and-persistent-sessions.md#starting-a-workspace).

- **About page.** A new persistent session starts on the about page too, so
  `runyte -a` before anything has been opened in that workspace shows it.
- **Several files.** Runyte opens every text file given and leaves the first
  active. Editor and ide launches show a stable `Opening workspace…` screen while
  they load.
- **Editable at once.** The first document frame is already editable. Syntax
  highlighting appears on its own when parsing finishes.
- **Caret position.** Put a one-based `+LINE` or `+LINE:COLUMN` immediately
  before a file to place its caret when that buffer is first shown. Columns
  count Unicode characters, not bytes:
  `runyte +12:4 "notes with spaces.md" src/main.rs`.
- **Same file twice.** Repeated spellings of the same resolved path share one
  buffer, at startup and later, in either mode. A save-as refuses a path
  already owned by another live buffer, including its resolved aliases; close
  that buffer before deliberately taking the path over.
- **Leading `-` or `+`.** Put `--` before such paths.
- **Binary files** open the external-program prompt described in
  [Opening binary files](files-and-buffers.md#opening-binary-files). Open them one at a time, so no
  explicit target can be silently skipped.
- **Standard input.** `runyte -` (piped/stdin scratch input) is deliberately
  deferred: Crossterm owns stdin for terminal events. Before `--`, a lone `-`
  exits with an explanation instead of starting a terminal that cannot tell
  file contents from input events.

Inside the editor, press `Space` then `?` for contextual help. See
[Help, hints, and the tutorial](the-screen.md#help-hints-and-the-tutorial).

<!-- navigation -->
[Contents](README.md) · [Previous: Overview](overview.md) · [Next: The screen](the-screen.md)
