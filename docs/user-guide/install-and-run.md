## Install and run

Runyte comes in a terminal edition and a desktop edition. Both have the same
editor, configuration, plugins and persistent sessions. The desktop edition
adds a window and media viewing, and it can also run in a terminal.

Both editions install as `runyte`. `--window` needs the desktop edition.
Clients and hosts in a persistent session must use the same Runyte version.

Start with the [README quick start](../../README.md#installation). The
[installation guide](../../INSTALLATION.md) covers choosing an edition,
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
Terminal. The rest of this guide applies to Windows too. This section lists
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
- Keeping unsaved editor state when the outer console window is closed.
  `Ctrl+C` and `Ctrl+Break` in the outer console ask for an orderly shutdown.
  After a close, Windows may end the process before cleanup finishes.
- Starting a detached persistent host from a job that forbids breakaway.
  `-a` and `--session-restart` report the refusal and leave no new host
  running. A restart checks this before stopping the old host. You can still
  attach to a host that is already running.

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
  Command lines use native Windows quoting. See
  [Windows terminals](terminals.md#windows-terminals).
- **`:pipe`** runs Windows PowerShell rather than `/bin/sh`. See
  [Shell pipes](files-and-buffers.md#shell-pipes).
- **`:quit-here`** needs the PowerShell 5.1 wrapper
  [runyte.ps1](../../contrib/runyte.ps1). See
  [change the shell directory on exit](workspaces-and-persistent-sessions.md#change-the-shell-directory-on-exit).
- **Git** is enabled when `git.exe` or `git.com` is found on an absolute `PATH`
  entry accepted by `PATHEXT`. Without Git, Git commands are disabled with a
  reason and editing still works. Restart Runyte after installing Git or
  changing `PATH`.
- **Paste:** Windows Terminal keeps `Ctrl-v` for its own paste action. Some
  versions send an empty paste event for an image, and Runyte recognizes it,
  even in the middle of a multi-key command. If the terminal sends nothing, use
  `Alt-v`. Bracketed paste arrives as text in every mode, so pasted lines never
  run editor commands.
- **Saves** replace existing files with `ReplaceFileW`, which keeps their
  metadata. Another program opening the file during a save can briefly get a
  missing-file or sharing error. The `wrote` message means the whole document
  is on disk.
- **Two hosts for one project** can run at the same time. The session
  manager and the session strip show them as separate rows. Selecting one by
  path or ID is refused as ambiguous, so use its name or pick the row.
- **`:session-clean`** is available inside the editor. On Linux and macOS use
  `runyte --session-clean`.
- **A bare `runyte`** with `mode: mux` attaches only to a project it can
  discover, including the exact home-directory fallback. Otherwise it refuses
  and creates nothing, as a bare `-a` does. `-a DIRECTORY` makes that directory
  a workspace. `--init` only creates one.
- **`--serve`** stops the host when the process that launched it exits.
- **Plugins** can request the `processes` capability for managed native helper
  processes.
- **`workspace.state_anchor`** chooses where retained state lives; see
  [Windows state anchor](configuration.md#windows-state-anchor).
- **External programs** for binary files open differently. See
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
  directory belongs to. Outside any workspace, the current directory becomes
  one if the file is inside it. In your home directory exactly, ide and mux use
  a home workspace and keep its state in the per-user state area. In a
  temporary or shared directory, in the filesystem root, or for a file
  elsewhere, ide refuses and suggests `runyte --init DIRECTORY`. Use
  `runed FILE` to edit without a workspace. See
  [Finding the workspace](workspaces-and-persistent-sessions.md#starting-a-workspace).

- **About page.** A new persistent session starts on the about page too.
  You'll see it after `runyte -a` if nothing has been opened in that workspace.
- **Several files.** Runyte opens every text file you give it and makes the
  first one active. Editor and ide launches show a stable `Opening workspace…`
  screen while they load.
- **Editable at once.** You can edit from the first frame. Syntax highlighting
  appears when parsing finishes.
- **Caret position.** Put `+LINE` or `+LINE:COLUMN` right before a file to place
  its caret when the buffer is first shown. Lines and columns start at 1.
  Columns count Unicode characters, not bytes:
  `runyte +12:4 "notes with spaces.md" src/main.rs`.
- **Same file twice.** Different spellings of the same path share one buffer,
  at startup and later, in either mode. A save-as refuses a path that another
  live buffer already owns. Close that buffer first if you really want to take
  over the path.
- **Leading `-` or `+`.** Put `--` before such paths.
- **Binary files** open the external-program prompt described in
  [Opening binary files](files-and-buffers.md#opening-binary-files). Open them
  one at a time, so that none is skipped without you noticing.
- **Standard input.** `runyte -` (piped/stdin scratch input) is not supported
  yet, because Crossterm owns stdin for terminal events. Before `--`, a lone
  `-` exits with an explanation.

When a workspace launch refuses, the message tells you:

- the mode, and whether a flag or the `mode` setting selected it
- the directory it searched
- for ide, why that directory was not made a workspace
- commands to open a project, create a workspace, or (in ide mode) edit without
  one

A directory argument to ide opens an explorer. To choose a different workspace,
use `cd DIRECTORY && runyte`. Mux can choose one directly with
`runyte --mux DIRECTORY`. Replace `DIRECTORY` with the directory you want, and
quote paths that contain spaces or shell characters.

### Graphical launchers

The macOS app and the Linux application-menu entry start in your home directory
in the configured `mode`. IDE and mux use the home workspace with integrated
terminals. `mode: editor` stays in editor mode. Close a mux window and launch
again to get back to the same persistent session. A host accepts one
interactive frontend at a time.

In IDE and mux, if the graphical launcher receives files, the first file's
directory chooses the workspace. A directory argument chooses that directory.
Editor mode opens files without a workspace, including new files whose parent
directory doesn't exist yet. Relative paths still start from the directory the
launcher was started in. Files open with the same caret, duplicate-path and
native-media rules as other window launches. Commands typed in a terminal still
choose the workspace from the current directory. Finder file-open Apple Events
are not integrated yet.

A startup refusal stays visible until you close the window. Invalid
configuration shows its error using default presentation settings. The
requested mode does not change, and workspace services do not start after a
refusal. If the system can't create a window at all, the error can only go to
stderr.

Inside the editor, press `Space` then `?` for contextual help. See
[Help, hints, and the tutorial](the-screen.md#help-hints-and-the-tutorial).

<!-- navigation -->
[Contents](README.md) · [Previous: Overview](overview.md) · [Next: The screen](the-screen.md)
