# Runyte user guide

This is the detailed reference for Runyte. For the project overview and quick
start, see the [main README](../README.md).

The guide is best read with an AI agent: ask it specific questions rather than
reading from top to bottom.

| Section | What it covers |
| --- | --- |
| [Overview](#overview) | What Runyte does, and how it relates to Helix |
| [Install and run](#install-and-run) | Installing, updating, Windows, and launching |
| [The screen](#the-screen) | Layout, status line, notifications, help, mouse |
| [Key bindings](#key-bindings) | Editing, Insert mode, search, layout, text objects, macros |
| [Files and buffers](#files-and-buffers) | Finder, explorer, directory tree, comparisons, pipes |
| [Panes](#panes) | Splits, focus, resizing, zen and fullscreen |
| [Terminals](#terminals) | Integrated terminal sessions |
| [Git](#git) | Status, branches, worktrees, history, staging, pull and push |
| [Language support](#language-support) | Syntax highlighting, rendered Markdown, language servers |
| [Workspaces and persistent sessions](#workspaces-and-persistent-sessions) | Standalone and persistent modes, the session manager |
| [Commands](#commands) | The command palette and every `:` command |
| [Plugins](#plugins) | Process plugins and application views |
| [Agent access to workspace context](#agent-access-to-workspace-context) | The context bridge for AI agents |
| [Diagnostics and logging](#diagnostics-and-logging) | The diagnostic log |
| [Configuration](#configuration) | The configuration file and every setting |
| [Custom key bindings](#custom-key-bindings) | Remapping keys and binding actions |
| [Themes](#themes) | Built-in and custom themes |

## Overview

### What Runyte does

**Editing**

- Normal, Insert, Replace, Select, and Command modes
- Multiple selections with Helix-style multi-cursor commands
- Counts, named registers, and recordable keyboard macros
- Rope-backed buffers with transactional, single-step multi-cursor undo
- Undo/redo, yank/paste, dirty-buffer protection, and save-as
- Operating-system clipboard yank and paste
- Structural Tree-sitter selection, text objects, outlines, syntax-aware
  indentation, and pane-local code folding
- Word completion from every open buffer, including the explorer, with no
  language server and no trigger key required
- Unicode-aware buffer positions and terminal widths

**Languages**

- Tree-sitter syntax highlighting for Python, Rust, Swift, C, C++, JavaScript,
  TypeScript, TSX, HTML, CSS, Go, Bash, Java, Kotlin, SQL, Lua, C#, Zig, CMake,
  Protobuf, Make, INI, Markdown, TOML, YAML, JSON, Dockerfile, XML,
  HCL/Terraform, Ruby, PHP, and Elixir
- Language servers: diagnostics, completion, hover, signature help, goto,
  references, rename, code actions, formatting, and symbol pickers
- Markdown documents rendered as formatted text on `?`, with terminal bold,
  italic, underline, and strikethrough

**Files and navigation**

- Oil-style editable directory explorer plus a unified Finder
- Editable directory buffers with explicit filesystem plan confirmation
- Filterable open-buffer picker and workspace-wide text search
- Multicursor search over the buffer or a selection, with literal and
  regular-expression flavours
- Per-pane jumplists with reversible cross-buffer navigation
- Registry-driven navigation, editing, search, and view-specific commands
- Side-by-side comparison of any two buffers, aligned line for line and
  scrolled together
- Binary files handed to a program you name, with recent choices remembered

**Layout and display**

- Arbitrarily nested vertical and horizontal splits with recency-aware
  directional focus
- Shared buffers between split panes
- Configurable Unicode-aware soft wrapping and visual-line movement
- Soft-wrap continuation arrows in the line-number gutter
- A centred, editable writing viewport toggled with `:zen`, and a plain
  maximized pane toggled with `:fullscreen`
- Which-key-style hints for every registered command family, plus exact
  completed-command descriptions and results on the interaction line
- Differential, flicker-free terminal rendering through Ratatui
- Fold-aware mouse cursor placement, drag selection, wheel scrolling, and pane
  resize

**Git**

- Asynchronous Git status, gutter marks, changed-file views, file/hunk/safe
  selected-line staging, commit, pull/push, branches, worktrees, history,
  blame, stashes, and diffs

**Terminals and sessions**

- Terminal panes running any interactive program — a shell, `htop`, `vim`, a
  coding agent — with scrollback, modal navigation over it, and a command that
  sends a buffer's selection to one as a single paste
- Optional `--persistent` local workspace persistence for unsaved editor,
  language-service, and live terminal-session state
- `$EDITOR`-compatible `--wait` requests with revision-safe local edits

**Configuration**

- YAML configuration
- A registry-backed settings browser with previews and atomic YAML updates
- Built-in and user-defined themes, browsable and configurable under `:theme`
- A bounded, searchable notification center under `:notifications` / `:not`

### Runyte and Helix

Runyte is not a Helix clone, and it is not a subset of one.

- **Shared:** the editing model and much of the keymap language come from
  Helix.
- **Added:** terminal multiplexing, an editable file manager, a detachable
  client-server host for unsaved buffers and live processes, rendered
  Markdown, image pasting, and a plugin runtime. None of these has a Helix
  counterpart.
- **Different:** a few Helix behaviors are deliberately absent, and a number
  deliberately differ.

The [key bindings](#key-bindings) section records what each binding does here.
`context/reference/helix-keymap-v1.md` is the register of which bindings match
Helix and which do not.


## Install and run

Runyte runs on Linux, macOS, and Windows 11. [Windows support](#windows-support)
lists what is missing or different there.

| Method | Platforms | Section |
| --- | --- | --- |
| Install script | x86-64 and ARM64 Linux and macOS | [Install and update with curl](#install-and-update-with-curl) |
| Release archive | x86-64 and ARM64 Linux and macOS; x86-64 Windows | [Manual download and source installation](#manual-download-and-source-installation), [Windows support](#windows-support) |
| `cargo install` | Anywhere Rust 1.88 and a C compiler are available | [Manual download and source installation](#manual-download-and-source-installation) |
| From a clone | Anywhere Rust 1.88 and a C compiler are available | [Building from a clone](#building-from-a-clone) |

### Install and update with curl

On x86-64 or ARM64 Linux and macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/runyte/runyte/main/install.sh | sh
```

Run the same command again to update.

**What the script does:**

1. Resolves the latest GitHub Release once.
2. Downloads that version's archive and `SHA256SUMS` over HTTPS.
3. Checks the archive's SHA-256.
4. Stages the executable and replaces `~/.local/bin/runyte`.

- A failed download, checksum, or extraction leaves an existing installation
  intact.
- The checksums establish integrity against the release manifest. They are not
  independent signatures.
- The [installer source](../install.sh) is maintained in this repository and
  served from `main`.

**Requirements:**

- No sudo.
- `curl`, `tar` with xz support, either `sha256sum` or `shasum`, and standard
  Unix utilities. Some Linux distributions package xz support separately as
  `xz-utils` or `xz`.
- Linux needs glibc 2.35 or newer. Alpine/musl and other operating systems or
  architectures are rejected.
- macOS binaries are unsigned and unnotarized. The script does not change
  Gatekeeper settings.

**Read the script before running it:**

```sh
curl -fsSL https://raw.githubusercontent.com/runyte/runyte/main/install.sh -o install.sh
less install.sh
sh install.sh
```

**Pin a version or choose a directory.** The script accepts a release version
(with or without `v`) and an absolute install directory:

```sh
sh install.sh --version 0.3.0 --install-dir "$HOME/.local/bin"
```

- Omit `--version` to install or update to the latest release.
- Keep the same `--install-dir` when updating a custom installation.
- A pinned version can reinstall or downgrade to any release with matching
  archives and checksums.
- The script refuses a symlink or directory at the destination `runyte` path.

**Add the directory to `PATH`** if it is not already there. For the default
location in a POSIX shell:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

**After installing:**

- Check `command -v runyte` and `runyte --version`, especially if another copy
  was installed through Cargo or manually. The installer only updates the copy
  in its chosen directory.
- Running editors and persistent sessions keep using the old executable until
  restarted. Save your work before restarting them.
- The script installs the executable only. It does not change configuration or
  shell startup files.
- To uninstall, remove `~/.local/bin/runyte` (or the file in your custom
  directory).

### Manual download and source installation

**Release archives.** Prebuilt archives for x86-64 and ARM64 Linux and macOS
are on the [GitHub Releases page](https://github.com/runyte/runyte/releases).

1. Download the archive for your machine and `SHA256SUMS`.
2. Compare the archive's checksum with its entry in `SHA256SUMS`: use
   `sha256sum` on Linux or `shasum -a 256` on macOS.
3. Extract it. The versioned top-level directory holds the editor, the
   configuration example, and complete licence material.

The macOS executables are unsigned and not notarized.

**From crates.io.** You need Rust 1.88 or newer and a C compiler; the bundled
tree-sitter grammars are built from C source during the install.

```sh
cargo install runyte --locked
runyte README.md
```

This installs the `runyte` editor executable. Keep `--locked`: without it,
Cargo ignores the dependency versions this release was tested with and
re-resolves to the newest compatible ones. That can pull in a crate needing a
newer toolchain than Runyte does, and the install then fails naming a
transitive dependency you have never heard of.

### Building from a clone

```sh
./build.sh --release
./target/release/runyte README.md
```

`build.sh` wraps `cargo build --bins` and forwards any extra arguments.

Install from the clone with Cargo:

```sh
cargo install --path .
```

During development:

```sh
./build.sh
cargo run -- src/main.rs
cargo test
```

### Windows support

Runyte runs natively on x86-64 Windows 11 24H2 or later, inside Windows
Terminal. The rest of this guide applies to Windows too; this section lists
what is missing, what is limited, and what works differently.

**Release ZIP.** Releases from 0.3.2 onward include an x86-64 Windows ZIP with
`runyte.exe`, the configuration example, and licence material.

- Its hash is in `SHA256SUMS`. Compare
  `Get-FileHash -Algorithm SHA256 <archive.zip>` with that entry before
  extracting.
- Executables are unsigned.
- The ZIP does not bundle the x64 Microsoft Visual C++ runtime
  (`VCRUNTIME140.dll`). Install Microsoft's x64 Visual C++ Redistributable if
  it is absent.

**Building from source** needs Rust 1.88 or newer and Visual Studio Build Tools
with the C++ toolchain and Windows SDK. The bundled grammars require the C
compiler.

```powershell
cargo build --release --locked
.\target\release\runyte.exe --standalone README.md
```

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
- Private runtime storage (logs, language-server approvals, the context bridge,
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
  [Windows terminals](#windows-terminals).
- **`:pipe`** runs Windows PowerShell rather than `/bin/sh`; see
  [Shell pipes](#shell-pipes).
- **`:quit-here`** needs the PowerShell 5.1 wrapper
  [runyte.ps1](../contrib/runyte.ps1); see
  [change the shell directory on exit](#change-the-shell-directory-on-exit).
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
- **A bare `runyte`** with `workspace.mode: persistent` attaches only to a
  project it can discover, and otherwise refuses without creating a workspace.
  `-a` initializes the current directory; `--init` creates an exact standalone
  workspace.
- **`--serve`** stops the host when the process that launched it exits.
- **Plugins** can request the `processes` capability for managed native helper
  processes.
- **`workspace.state_anchor`** chooses where retained state lives; see
  [Windows state anchor](#windows-state-anchor).
- **External programs** for binary files open differently; see
  [Opening binary files on Windows](#opening-binary-files-on-windows).

### Starting Runyte

| Command | What opens |
| --- | --- |
| `runyte` | The read-only about page |
| `runyte .` | The current directory in the explorer |
| `runyte file.txt` | That file |
| `runyte a.md src/main.rs` | Both files; the first is active |
| `runyte +12:4 notes.md` | `notes.md` with the caret on line 12, column 4 |
| `runyte -- +draft.md -notes.md` | Files whose names start with `+` or `-` |
| `runyte --help` | Command-line options |

Details:

- **About page.** A new persistent session starts on the about page too, so
  `runyte -a` before anything has been opened in that workspace shows it.
- **Several files.** Runyte opens every text file given and leaves the first
  active. Standalone launches show a stable `Opening workspace…` screen while
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
  [Opening binary files](#opening-binary-files). Open them one at a time, so no
  explicit target can be silently skipped.
- **Standard input.** `runyte -` (piped/stdin scratch input) is deliberately
  deferred: Crossterm owns stdin for terminal events. Before `--`, a lone `-`
  exits with an explanation instead of starting a terminal that cannot tell
  file contents from input events.

Inside the editor, press `Space` then `?` for contextual help. See
[Help, hints, and the tutorial](#help-hints-and-the-tutorial).


## The screen

### Screen layout

```text
 session strip (optional, persistent sessions only)
┌ pane title ───────────────┐┌ pane title ───────────────┐
│ gutter │ buffer viewport  ││ gutter │ buffer viewport  │   editor area
│        │                  ││        │                  │
└───────────────────────────┘└───────────────────────────┘
 global status line
 interaction line
```

| Name | What it is |
| --- | --- |
| **Runyte screen** | The complete terminal surface. |
| **Editor area** | The upper part, holding one or more panes. |
| **Pane** | A **pane border**, a **pane title** in its top border, and a **pane body** inside it. |
| **Gutter** | Part of the pane body: line numbers, soft-wrap markers, syntax-fold markers, and Git change marks. |
| **Content padding** | Optional space in the pane body, used by aligned generated pages. |
| **Buffer viewport** | Where buffer text is shown and, for editable buffers, changed. |
| **Global status line** | Mode, workspace directory, active-buffer state, cursor, Git/LSP state, and unread notifications. |
| **Interaction line** | An active prompt or the last action echo. |

These names are also the vocabulary of help and source documentation, and any
future extension surface must inherit them. The canonical definitions live in
[`context/reference/ui-vocabulary.md`](../context/reference/ui-vocabulary.md).

**Which surface a task gets** depends on its lifetime and interaction, not on
the shape of its box:

| Surface | Used for |
| --- | --- |
| **Buffer** (or a pane-backed filterable list) | A navigable result. Keeps movement, selection, search, splits, help, and buffer management. |
| **Special buffer** | Contents Runyte assembles rather than reads as file text: the explorer, config, Git views, notifications, help, the about page. |
| **Picker overlay** | A transient choose-one request. |
| **Context overlay** | Assistance tied to a source position. |
| **Confirmation overlay** | A pending prepared operation. |
| **Interaction line** | Short scalar input. |
| **Input overlay** | Richer validated input. |

Special buffers:

- remain full buffers while displayed, may be shared by panes, and may be
  editable or read-only;
- are retained while clean, up to the eight most recently active. Activating a
  ninth retires the least recent one once it is detached;
- stay available while dirty, until saved or discarded.

A pathless scratch buffer is ordinary text, not a special buffer.

For example, `Space g l` opens a Git-log buffer for ordinary browsing, while
`Space g f` opens a fuzzy commit picker that disappears after one choice. Both
open the same retained special commit-detail buffer when a commit is selected.

**Pane titles** name the buffer kind:

- ordinary paths are prefixed `[file]`;
- directory paths are prefixed `[explorer]`;
- virtual views keep their bracketed names, such as `[git status]` and
  `[git branches]`;
- `[+]` marks unsaved changes and `[RO]` a read-only buffer.

### Global status line

```
 NOR │ standalone │ Workspace: /home/me/code/runyte [+]   412:17 · 34% │ 3 sel │ main ~1 │ rust-analyzer 0E 2W
```

| Part | Meaning |
| --- | --- |
| `NOR` | The mode, in the current mode's caret colour (see [Mode carets](#mode-carets)). The rest of the row keeps the theme's ordinary background. |
| `standalone` | The workspace mode. `standalone` keeps live state in the TUI process; `persistent` keeps it in a separate local process. These are the values of `workspace.mode`. |
| `Workspace: …` | The current workspace directory. When it does not fit, the beginning is replaced with `...` and the identifying end kept. |
| `[+]` / `[STALE]` / `[RO]` | The active buffer has unsaved changes / the path it shows (a file or an explorer's directory) disagrees with the accepted baseline / is read-only. |
| `412:17 · 34%` | Cursor line and column, and how far through the buffer it is. |
| `3 sel` | Selection count, shown above one. |
| `main ~1` | Git branch and outstanding changes. See [Status line and gutter](#status-line-and-gutter). |
| `rust-analyzer 0E 2W` | Language-server summary. |
| `E1 W2 I3` | Unread notification counts, in their semantic theme colours. At narrow widths they compact to the highest severity plus the total, such as `E4`. |

Fields after the position appear only when they apply. Pane titles carry file
and buffer identity. The active theme is not shown; `Space o t` shows and
changes it.

**The percentage:**

- The first line reads `0%` and the last `100%`. No line between them does: a
  cursor one rounding step from either end reads `1%` or `99%`.
- A one-line buffer reads `100%`.
- It follows the cursor, not the topmost visible line, so `Z j` and the other
  view-scroll keys leave it alone until the cursor moves.

### Interaction line

The interaction line teaches each command as you use it:

| Result | Echo |
| --- | --- |
| No more specific result | The typed keys and the command description: `g l (Move to line end)` |
| A useful result | The result: `Space e (opened /path/to/dir)`, `:c (closed file)` |
| Failed | `binding (action · failed: message)` |
| Unavailable | `binding (action · unavailable: message)` |
| Unbound keys | `No binding: g z` |

- An active prompt uses the line temporarily. Errors, warnings, service output,
  and other notifications never replace the prompt or the echo.
- A failure carries the outcome's own message, not only the fact that it
  failed.
- A message longer than the space left, or spanning several lines, is cut to
  fit and ends with `...`. `:not` keeps the complete text. A prompt being typed
  is never cut, since its cursor position depends on the full text.
- A mistyped sequence (`No binding: g z`) is also shown in the key hints, but
  is not kept as a notification: it says nothing worth reading later, and a
  burst of mistyping would otherwise be the largest and only unbounded source
  of unread notifications.

**Pasting into prompts.** Single-line prompts and filters reject a whole paste
that contains control characters, including a trailing newline or tab.

- The text, cursor, and selected result stay unchanged, and the prompt shows
  `Control characters are not allowed; nothing was inserted` until the next
  input.
- This applies to the command line, search and rename prompts, Finder and list
  queries, session-directory queries, and typed Git confirmations.
- Literal backslash sequences such as `\n` are ordinary prompt text.
- Buffers and terminal sessions still accept multiline paste. Terminal paste
  actions, including macOS `Cmd-v`, keep line breaks even when the terminal
  sends them as carriage returns, and keep existing LF and CRLF separators.

### Notifications

`:notifications` (alias `:not`) opens `[notifications]`: one searchable,
read-only buffer with the retained history, newest first.

Each entry has:

- a local `YYYY-MM-DD HH:MM:SS` timestamp,
- an `ERROR`, `WARNING`, or `INFO` severity assigned by Runyte,
- a source, a title, and retained multiline details.

**Reading and unread counts:**

- Opening the buffer acknowledges the entries retained at that moment.
- New notifications arrive without moving focus and are unread.
- Consecutive identical notifications merge: their timestamp and occurrence
  count update and they become unread again.

**Severity** describes the underlying condition, not whether the action
completed:

| Severity | Examples |
| --- | --- |
| `INFO` | Expected context refusals, unavailable actions, empty search results |
| `WARNING` | Refusals that protect data or consistency, such as saving a read-only buffer or a file that changed on disk |
| `ERROR` | A failure in Runyte or an external dependency: a failed Git command, host operation, file write, or language-server exchange |

So the interaction line can say an action `failed` while its notification is
`INFO`.

**Bounds:**

- The configured entry count (`notifications.history_limit`), plus at most
  1 MiB per notification and 8 MiB per workspace.
- Git failures keep labelled stdout and stderr within the 1 MiB budget, enough
  for thousands of ordinary lines, and add a visible truncation marker when a
  hostile hook exceeds it. The rest of an overlong stream is still drained
  while the subprocess runs, so keeping less text never breaks an otherwise
  successful hook or helper.
- Successful asynchronous Git output updates the action echo while it is still
  current. Multiline output, or output whose echo has been superseded, is kept
  as `INFO`.

### Help, hints, and the tutorial

| Command | What it opens |
| --- | --- |
| `Space ?` | Contextual help for the current buffer type |
| `:help` or `:?` | The general Runyte manual |
| `:help <topic>` | The manual at a section, such as `:help regex`, `search`, `mouse`, `git`, `sessions`, or `lsp` |
| `:about` | Logo, version, and a short getting-started guide |
| `:tutorial` | A guided, hands-on introduction |
| `:grammar` | The active editing grammar |

#### Key hints

Press a prefix such as `g`, `Space`, or `Ctrl-w` and a popup lists what can
follow.

- The default grammar groups commands under labelled `Space` namespaces. The
  popup shows one namespace row at a time, then the namespace's entries when
  you enter it.
- Short keys stay as fast or compatibility bindings. A row naming two keys,
  such as `Space s a, &`, runs the same command either way, so the namespace
  teaches the short spelling rather than hiding it.
- Bindings added by plugins or configuration under an unlabelled prefix fold
  the same way: several bindings under `Space =` appear as one `Space =` row,
  named after their shared command name or by how many there are.
- `Space l` (**Language (LSP)**), `Space x` (**Syntax (Tree-sitter)**), and
  `Space g` (**Git**) are dimmed with a reason when the service is not ready:
  no language server, no parser, or no Git / no repository. They stay
  navigable so each command can explain itself; LSP status and restart can
  stay usable without an attached document.

In the popup:

- `Ctrl-n` and `Ctrl-p` scroll without affecting the pending keys. In Normal
  and Select, Up and Down scroll too, unless that arrow completes a binding.
- Scroll controls appear in the title when the entries do not fit.
- Up to three columns are used when complete rows fit. Standalone and attached
  persistent clients use the same layout.

#### Contextual help (`Space ?`)

- One document per buffer type, not per mode. Normal and Select bind the same
  keys, so a text buffer has a single `TEXT` document describing both.
- In a plugin view it lists the actions Tab offers there. A plugin that
  supplies help topics also explains the view's workflow, such as browsing a
  table's rows or inspecting one record.
- Key lists are read from the keymap when help opens, so they follow your
  configured bindings. In a read-only view, keys that would only report a
  refusal are left out.

The generated key lists cover each mode:

- **Normal and Select:** buffer-specific keys and `Tab` actions first, then
  prefixes whose hint popup teaches the rest, then direct keys grouped as
  letters and punctuation, `Ctrl` chords, `Alt` chords, and named keys.
  Shifted `<` and `>` also carry searchable `Shift-<` and `Shift->` spellings.
- **Insert and Replace:** shared keys once, then separate lists for keys that
  differ. A read-only view says when these modes are unavailable. Terminal
  Insert lists keys sent to the child and Runyte-owned exceptions separately.
- **Command:** prompt controls under their own heading. The command prompt
  handles these itself, outside the keymap.

#### Help buffers

- Both kinds of help are ordinary read-only buffers: they scroll, search,
  split, and close with `q`, `:c`, or `Space b c`. Nothing is truncated to fit
  the window, and opening one kind does not replace the other.
- The about page and help colour section titles, commands, key bindings, file
  paths, web links, and technical examples with the theme's syntax colours.
  They are not Markdown: visible backticks stay ordinary searchable, copyable
  text, and colouring changes no buffer coordinate and makes nothing
  clickable.
- A read-only buffer is marked `[RO]` in the pane title and the status line,
  and its help says so: `Help · RUNYTE · GIT STATUS · Read-only`. The status
  line describes the buffer on screen; the help title describes the buffer
  type it is about, so help for an editable type is not titled read-only.

#### The about page

`:about` is centred against the pane, not against fixed columns in its text.
Resize the window or split the view and the page moves, without a character
being rewritten. The space around it is drawn, not stored, so search,
scrolling, and clicks land on the text. A page taller than its pane starts at
its first row and scrolls like any read-only buffer.

#### The tutorial

`:tutorial` opens two ordinary panes: read-only instructions and disposable
scratch text.

- **First choice:** show Vim-like motion spellings, Helix-like ones, or both.
  This changes only the spellings in the tutorial, never Runyte's behavior or
  keymap.
- **Lessons:** modes; selection-first editing with characterwise and
  `x`/`X` whole-line selections; search; multiple carets; `Space` discovery;
  `Ctrl-w` pane commands.
- **Views:** scratch, generated, and editable explorer buffers versus terminal
  pane content. You open the explorer, return with `Alt-o`, create a terminal,
  and close that terminal session through its manager.
- **Final lessons:** `Ctrl-o`/`Ctrl-i` jump history, the standalone/persistent
  boundary, and pointers to `:help` and `Space ?`.

| Command | Effect |
| --- | --- |
| `:tutorial` | Resume the live lesson |
| `:tutorial reset` | Start over |
| `:tutorial sessions` | Open the persistent-session lesson directly |

#### Editing grammar

`:grammar` reports the active Runyte grammar. `helix` is accepted as a
configuration and command alias for `runyte`.

### Mouse

Mouse support needs a terminal with mouse reporting and `editor.mouse: true`
(the default).

| Action | Effect |
| --- | --- |
| Click | Focus a pane and place the caret |
| Shift-click | Extend the selection |
| Drag | Select, entering Select mode |
| Drag past the top or bottom row | Scroll and keep extending |
| Wheel | Scroll the pane under the pointer |
| Drag a shared border | Resize the split |
| Right-click a selection | Copy all selections to the system clipboard, like `Space c y` |

- **Autoscroll** follows visual rows, including soft wrapping and collapsed
  folds. Move back inside the pane or release the button to stop.
- **Right-click copy** leaves the selections alone and reports
  `right mouse click (yanked to system clipboard)`.
- **Character rule.** The pointer names the character under it, not the gap
  before it, so a drag covers the characters it started and ended on, in
  either direction. Pressing past a line's end places the caret on its last
  character, or past it in Insert mode, as keyboard motion does.
- **Terminal panes.** A click focuses a live terminal in Insert mode; a
  reviewed terminal stays in Normal/review until a terminal insert key returns
  to the live screen. A left drag in review selects cells and enters Select
  mode, and right-click copies that selection. Clicking another pane focuses
  it in Normal mode. Dragging from a document press still selects and enters
  Select mode.
- Pointer coordinates go through the same fold- and wrap-aware rows used for
  drawing, so collapsed lines and wide Unicode glyphs never disagree.
- Passive pointer motion never clears key hints or status and does not
  redraw.
- Mouse capture is released on every exit and error path.

**Native text selection.** Capturing the mouse takes over the terminal's own
text selection. To keep that instead, set `editor.mouse: false` and restart.

### Jump history

| Key | Moves through |
| --- | --- |
| `Ctrl-o` / `Ctrl-i` | Every recorded position, including terminal surfaces and positions within one file |
| `Alt-o` / `Alt-i` | The same history, stopping only on a different buffer or terminal surface |

`Alt-o` leaves a document you have read through in one press, rather than one
per section.

Closing a buffer with `:c` or `Space b c` keeps every pane. Each pane returns
to its own most recently used live buffer, or gets a new scratch buffer when
none remains.

### Contextual actions (`Tab`)

In Normal and Select modes, `Tab` asks what can be done with the thing under
the cursor.

- **Git views** open an action menu. Each row has four aligned columns: the
  mnemonic, one word naming the action, whether it acts on the row or the whole
  buffer, and a sentence explaining it.
- **Language-server buffers** request code actions and open the code-action
  picker, or report that none are available.
- **Other views** (explorer, lists, plugin views) offer their own actions.

In an action menu:

| Key | Effect |
| --- | --- |
| Arrows or `j` / `k` | Move |
| Enter | Run the selected action |
| The mnemonic | Run that action directly |
| Escape | Cancel |

Mnemonics belong only to the open menu, so `s`, `n`, `p`, and the rest keep
their normal meanings in every buffer.


## Key bindings

The spellings in this guide are Runyte's defaults. The `keys` configuration
section can move the application and window prefixes, their descendants, and
advertised aliases; see [Custom key bindings](#custom-key-bindings). Live help,
hints, About, tutorials, and action messages always show the effective
spelling.

Other sections document the keys of their own views: [Files and
buffers](#files-and-buffers), [Panes](#panes), [Terminals](#terminals),
[Git](#git), [Language support](#language-support), and [Workspaces and
persistent sessions](#workspaces-and-persistent-sessions).

### Editing

The one-key rows in this table are the complete direct-binding inventory for
Normal and Select modes. Common prefixed gestures follow for context. Explorer
keys are under [Directory buffers](#directory-buffers), and Insert-mode keys
under [Insert and Replace modes](#insert-and-replace-modes).

| Key | Action |
| --- | --- |
| `Shift-Left` / `Shift-Right` | Visit the previous / next running persistent session (persistent mode) |
| `h` / `j` / `k` / `l`; `Left` / `Down` / `Up` / `Right` | Move left, down, up, right |
| `w` / `b` / `e` | Next word / previous word / word end |
| `W` / `B` / `E` | Long-word variants |
| `f` | Find the next typed character in the buffer |
| `F` / `t` / `T` | Find backward, or move until a character forward / backward |
| `Home` / `0`; `^`; `End` / `$` | Start / first non-whitespace / end of line |
| `Ctrl-b` / `Ctrl-f`; `Ctrl-u` / `Ctrl-d` | Page up / down; half-page up / down |
| `PageUp` / `PageDown` | Page up / down |
| `gg` / `ge` or `G` | Start / end of file |
| `gp` / `gP` | Next / previous paragraph |
| `gf` | Open the selected path exactly, or infer the complete path under a bare cursor; inferred paths at the end of a sentence ignore trailing punctuation if the literal name does not exist. On a Markdown link to `#heading` or `file.md#heading`, land on that heading |
| `gw` | Dim the view, label nearby words with one key and farther words with two, then type a label to jump |
| `gt` / `gc` / `gb`; `H` / `M` / `L` | Move to the top / center / bottom of the visible window |
| `i` / `a` / `I` / `A` | Insert before/after cursor or at line boundary |
| `o` / `O` | Open line below / above |
| `r` / `R` / `~` | Replace once / enter Replace mode / toggle case |
| `v` | Enter Select mode |
| `x` / `X` | Select whole lines and walk down / extend whole lines above |
| `%` | Select the entire buffer |
| `C` / `Alt-C` | Add a cursor on the nearest line below / above holding a character at the cursor's column, skipping the ones too short |
| `V` / `Alt-V` | Add a cursor on the next / previous line, padding short or empty lines with spaces to the same display column |
| `;` / `Alt-;` | Collapse selections / flip their direction |
| `,` / `Space s c` / `Alt-,` | Keep only the primary selection / drop it |
| `)` / `(` | Make the next / previous selection primary |
| `Alt-)` / `Alt-(` | Rotate selection contents forward / backward |
| `&` | Pad until every cursor shares the rightmost display column |
| `_` | Delete trailing whitespace from every selected line; `%` then `_` strips the buffer |
| `Alt-_` | Shrink every selection past the whitespace at its ends, without changing the text |
| `Space p .` | Toggle dim `·`, `→`, and `¬` markers for spaces, tabs, and line endings |
| `d` / `c` | Delete / change selection or cursor character; `d` after transient `x`/`X` cuts whole lines |
| `y` / `p` / `P` | Yank selection or cursor character, leaving a caret / replace the selection, or paste after a bare caret / paste before |
| `Y` | Yank every line the selection touches, as whole lines, leaving a caret |
| `>` / `<` | Indent / unindent |
| `J` | Join the selected lines with a space, or pull the line below up to a single-line selection or caret |
| `Ctrl-c` | Comment or uncomment every line the selection touches, using the buffer language's line comment; also bound in Insert mode |
| `u` / `U` | Undo / redo |
| `Alt-u` / `Alt-U`; `Space s u` / `Space s U` | Undo / redo selection changes, back to the last text edit |
| `s` / `/` | Search with an escaped literal, ignoring case / with a regular expression |
| `n` / `N` | Step to the next / previous match |
| `*` | Select every occurrence of the word or selection under the caret |
| `Ctrl-o` / `Ctrl-i`; `Alt-o` / `Alt-i` | Jump backward / forward through every navigation point; jump backward / forward to another buffer |
| `Tab` | Open contextual actions for the selection or row under the caret |
| `Ctrl-s` | Save |
| `Ctrl-v` / `Alt-v` | Paste the system clipboard, storing an image in the workspace and writing a numbered Markdown link to it; also bound in Insert mode |
| `:` | Open the command palette |
| `\|` | Shell pipe key (reserved; use `:pipe <shell-command>`) |
| `<n>` before a command | Repeat a motion or countable command |
| `<n>gg` / `<n>G` | Go to line `<n>` |
| `"` then a register | Select a named register; uppercase appends and `_` discards |
| `Space m …` | Record, replay, and list macros; see [Macros](#macros) |
| `mm` | Jump to the matching bracket |
| `m i …` / `m a …` | Select inside / around a word, paragraph, delimiter pair, function, type, or argument; see [Text objects](#text-objects) |
| `z…` / `Z…` | View alignment and scrolling |
| `Esc` / `Ctrl-\` (`Ctrl-4` on legacy terminals) | Return to Normal mode |

#### Selecting whole lines

| Key | Command | Effect |
| --- | --- | --- |
| `x` | `select-line` | Select the current line, then walk down |
| `X` | `extend-line-above` | Expand a partial selection to whole lines, then grow upward without dropping lines already selected |
| — | `extend-line-below` | Grow the lower edge |
| — | `select-line-up` | The former upward edge-walking behavior, for custom bindings |

For example, `x x X` adds the line above the two selected lines. All four
support counts and multiple selections.

#### Selection history

`Alt-u` (`:selection-undo`) brings back a selection lost to a motion, Escape,
a collapse, or removing extra cursors. `Alt-U` (`Alt-Shift-u`,
`:selection-redo`) reapplies it. `Space s u` and `Space s U` are the same
commands.

- Every range, its direction, the primary range, and the selection mode come
  back, including `x`/`X` whole-line behavior. Text never changes.
- A counted motion, an accepted search, or a mouse drag is one step. Cancelled
  search previews add none. A new selection change clears redo.
- History is per pane and per buffer, works in read-only buffers, and survives
  switching buffers.
- Any text change, including undo/redo or an edit in another pane, starts a
  new history.
- Restoring never enters Insert/Replace mode or reopens a prompt.
- Limits: 128 states and 4,096 total ranges per history, for up to 32 buffers
  per pane. An oversized selection ends that history.
- Terminal review is not supported.

#### Selecting motions

In Normal mode, `w`, `b`, `e`, `W`, `B`, `E`, `f`, `t`, `F`, and `T` select the
text they cross, as in Helix:

| Keys | Result |
| --- | --- |
| `w d` | Delete a word and the space after it |
| `e c` | Change to the end of the word |
| `w p` | Replace what `w` selected — the word and the space after it — with the register |

- `w` selects through the whitespace after the word, `e` through the end of the
  next word, and `b` back to the start of the previous one.
- `f` selects from the caret through the found character, `t` up to it.
- Each press starts a fresh selection, which never contains a line break.
- Anything that reads a selection reads this one: `p` replaces it, `s` and `/`
  search inside it, and `*` searches for it.
- Other motions still move a caret, and Select mode still extends.

Set `editor.selecting_motions: false` to make these motions move a caret too.
A word is then selected with `v` first: `v e d` deletes to the end of the word.

#### Yank and paste

**Yank ends the gesture.** `y` and `Y` return to Normal mode and collapse each
range to a caret on the last character copied, one caret per selection.

- That makes `y` then `p` a duplicate: the caret pastes past the text just
  yanked. A range that outlived the yank would be invisible in Normal mode
  and would make `p` replace the yanked text with itself.
- `Space c y` is the exception. It is also what a right click does, so it
  copies without disturbing the selection.

**`p` depends on the selection**, as `d` and `c` do:

| Selection | `p` does |
| --- | --- |
| A bare caret | Paste after the caret |
| A range holding text | Replace that text |
| A bare caret, linewise register (from `x y` or `Y`) | Paste whole lines |
| A characterwise selection, linewise register | Replace exactly the span, with the register's final line ending removed; inner line breaks stay |
| An `x`/`X` or Vim line selection, linewise register | Replace whole lines |

- The register is not used up and the replacement stays selected, so the same
  content can be pasted over one range after another.
- A multi-selection from a search replaces every match at once.
- `P` never replaces. It is the way to paste at the start of a selection
  without giving it up.
- `Space c p` and `Space c P` follow the same rules from the system clipboard.

#### Pasting images

`Ctrl-v` and `Alt-v` paste from the system clipboard in Normal, Select,
Insert, and Replace modes. When the clipboard holds an image:

1. The image is stored under `.runyte/cache/images/` in the workspace, named
   by the hash of its content.
2. The document gets a numbered Markdown link, such as
   `[Image 1](.runyte/cache/images/1f0a2b3c4d5e6f70.png)`.

- Pasting the same screenshot twice stores one file.
- The number continues past the highest `[Image N]` already in the document,
  rather than counting links.
- `?` renders the link as **Image 1**, without the path.
- A path with a space or a parenthesis is written in angle brackets,
  `[Image 1](<My Notes/a (copy).png>)`, the only spelling that survives being
  read back.

**When text is pasted instead.** A clipboard with no image, a machine with no
helper that can supply one, or an unresponsive display server all paste text,
exactly like `Space c p`.

A clipboard offering text *as well as* a picture counts as text. Copying
spreadsheet cells or formatted text attaches a rendered bitmap for "paste as
picture" commands, and taking it would lose the text you selected. A copied
picture has no text beside it, which is what screenshot tools and a browser's
"Copy Image" produce. If `Ctrl-v` pastes text where you expected an image, the
source is offering text. Once the clipboard has said it holds an image, failing
to fetch it is reported rather than silently pasting text.

Both keys are unbound in Terminal Insert, where they reach the program if the
outer terminal passes them through.

**The cache is scratch.** `.runyte/` is not tracked by Git, so a pasted image
travels with the working copy, not the commit.

- A document read from another clone needs its images moved somewhere the
  repository carries, and its links updated.
- Nothing promises to keep a file there. A document meant to outlive the
  session should not point into it.

**Storage safety:**

- Directories and images are private: `0700` and `0600` on Linux and macOS,
  owner-only permissions on Windows local NTFS.
- Symlinked or reparse-point storage paths, hard-linked entries, and files
  whose bytes disagree with their content-hash name are refused.
- Writes use exclusively created temporary files and atomic publication.

**On Windows:**

- Registered PNG data is read directly; supported native bitmap data is
  converted to PNG. Plain text still takes precedence.
- Clipboard data and converted images are limited to 64 MiB, and bitmap
  dimensions must fit in 64 MiB of RGBA pixels.
- Unsupported compressed bitmap layouts and custom colour profiles report an
  error.
- The editor waits at most one second for the clipboard. If Windows takes
  longer, another request reports the clipboard as busy; retry after the
  worker finishes.

<a id="insert-mode"></a>

### Insert and Replace modes

| Mode | Entered with | Typing |
| --- | --- | --- |
| Insert | `i`, `a`, `I`, `A`, `o`, `O`, `c` | Adds text at every caret |
| Replace | `R` | Overwrites the character ahead of each caret |

**Replace mode details:**

- Entering collapses every selection to its active head.
- A caret at line end appends instead of overwriting.
- A newline inserts a line break rather than consuming the existing one.
- Unicode characters are replaced one for one, and CRLF stays one line ending.
- The whole Replace session is one undo checkpoint.
- Lowercase `r` is the single-character Normal-mode command and never enters
  Replace mode.

The keys shared by Insert and Replace modes:

| Key | Action |
| --- | --- |
| `Shift-Left` / `Shift-Right` | Visit the previous / next running persistent session (persistent mode) |
| `Esc` / `Ctrl-\` (`Ctrl-4` on legacy terminals) | Return to Normal mode |
| `Backspace` / `Shift-Backspace`; `Delete` | Delete the previous / next character |
| `Alt-Backspace` / `Alt-Delete` | Delete the previous / next word |
| `Ctrl-u` / `Ctrl-k` | Delete to the start / end of the line |
| `Enter` / `Ctrl-j` | Insert a newline with the current indentation and optional smart indentation |
| `Tab` / `Shift-Tab` | Insert the configured indent style / the other style (`spaces` to the next tab stop, or one tab) |
| `Left` / `Down` / `Up` / `Right` | Move the caret |
| `Home` / `End`; `PageUp` / `PageDown` | Move to a line boundary; move by a page |
| `Ctrl-x` | Ask the language server for completions |
| `Ctrl-c` | Comment or uncomment the lines holding the carets |
| `Ctrl-s` | Save |
| `Ctrl-v` / `Alt-v` | Paste the system clipboard, storing an image in the workspace and writing a numbered Markdown link to it |
| `Ctrl-w` then a pane suffix | Move to another pane without first leaving Insert or Replace mode |

**Deleting in Replace mode.** Backspace or Shift-Backspace retraces the current
overwrite run: overwritten characters come back, and characters appended past
line end are removed. Alt-Backspace and `Ctrl-u` restore by word and to the
start of the line.

**Shift-Backspace** in Insert mode deletes like Backspace, so Shift can stay
held while correcting uppercase text.

**Leaving.** Escape or `Ctrl-\` returns to Normal mode.

#### Commenting lines

`Ctrl-c` comments or uncomments every line the selection touches, using the
buffer language's line comment.

| Marker | Languages |
| --- | --- |
| `//` | Rust, C, C++, C#, Go, Java, JavaScript, TypeScript, TSX, Kotlin, Swift, Zig, Protobuf, PHP |
| `#` | Python, Bash, TOML, YAML, CMake, Make, Dockerfile, HCL, Ruby, Elixir |
| `--` | SQL, Lua |
| `;` | INI |
| none | CSS, HTML, JSON, XML, Markdown — the key reports this and changes nothing |

- The marker goes at the least-indented line, so nested lines keep their
  relative indentation.
- Blank lines are left alone both ways.
- Uncommenting removes the marker and at most one space after it: `// x` and
  `//x` both become `x`.
- A partly commented block becomes fully commented first, so a second press
  always undoes the first.
- In an extensionless script whose language comes only from its shebang, the
  shebang line is left unchanged, so the buffer keeps its language.
- In Insert mode `Ctrl-c` acts on each caret's own line. Entering Insert mode
  collapses selections to carets, so choose a block in Normal or Select mode.

#### Auto-closing pairs

`editor.auto_close` is off by default. Turn it on in `Space o o` to pair typed
`()`, `[]`, `{}`, single quotes, and double quotes in Insert mode.

- The caret stays inside the pair. Typing the closer already under the caret
  moves past it.
- Backspace between an empty pair removes both characters, including pairs
  already in the document.
- Openers pair before whitespace, closing punctuation, or the end of the
  buffer.
- Quotes also need a non-word character before them. Escaped quotes and
  apostrophes inside words stay literal.
- It is a local text heuristic, independent of language syntax.
- Paste, Replace mode, prompts, and terminal input stay literal.
- Each caret is handled independently.

#### New lines and indentation

| Key | Keeps | Adds |
| --- | --- | --- |
| `o` / `O` (Normal) | The row's exact leading spaces and tabs | Nothing: no list marker, no syntax level |
| Enter (Insert) | The row's exact leading spaces and tabs | At most one level in `editor.indent` style when the syntax indentation query asks for it; a grammar that requires a tab still gets a tab |

`o` and `O` place the caret after the indentation, and work the same with
smart newline off.

**Markdown lists** continue on Enter while `editor.smart_newline` is on (the
default):

- bullets keep their marker, numbered and lettered items advance, and task
  items start unchecked;
- Enter on an empty item ends the list;
- Backspace right after a marker turns it into a continuation indent, whether
  or not text follows; another Backspace removes that alignment in one press;
- each of these edits renumbers the following numbered or lettered items that
  were in sequence, skipping nested items, continuation lines, and blank
  lines. A list numbered `1.` throughout, or one with a gap, keeps its numbers
  from that point on;
- a single `I.` or `V.` advances as a letter unless the previous sibling
  establishes Roman numbering.

In other file types, smart newline keeps the alignment under a list item's
content. With `editor.smart_newline: false`, Enter keeps only the row's
leading indentation.

Unsupported, malformed, oversized, and unterminated-final-line cases keep the
exact prefix and never block the newline.

### Search

Search has two flavours and one behavior:

| Key | Flavour |
| --- | --- |
| `s` | Literal text, ignoring case. The pattern is escaped, so `foo(` and `a.b` find themselves. No wildcards. |
| `/` | A regular expression, matching case unless the pattern says otherwise. Use it for wildcards or case-sensitive search. |

There is no separate case-sensitive literal key: use `/`, escaping any regex
syntax, when case matters, and `(?i)` in a `/` pattern to ignore case.

#### How a search selects

Every search selects *all* its matches at once, so the next edit applies to
all of them.

- Each match is selected in full, with the cursor on its last character, where
  an append or a motion continues from.
- The primary match has the primary-selection ground and one cursor in the
  Select colour (pink in `ember-dark` and `ember-light`, orange in most other
  built-in themes). Other matches keep the secondary selection colour without
  cursors.
- The status line shows the primary's position among the results.

**Then:**

| Next action | Effect |
| --- | --- |
| An edit straight away | Changes every match |
| `n` / `N` | Selects only the next / previous match; further presses cycle that single selection through the remembered results |
| An edit after `n` / `N` | Changes only the match you reached |
| A selection motion such as `e` | Turns the results into ordinary selections, with their Select-coloured endpoint cursors, and shows the selection count |

#### Preview while typing

- Every match the pattern would select is highlighted as Enter would leave it.
- The pane scrolls to the match that would become primary: the first at or
  after the cursor.
- Nothing is selected until Enter. Escape returns the pane to where it was.
- A pattern matching nothing highlights nothing and returns the view.
- An unfinished regular expression such as `foo(` keeps the last preview until
  it is valid again.
- The rest of the pane stays dimmed under the prompt.

#### Searching inside a selection

When at least two characters are selected — from `v` and a motion, `x`, `%`,
or a previous search — the search looks only inside that text, and `n`/`N`
wrap within it.

Searches therefore narrow step by step:

1. `x x x` selects three lines.
2. `s` finds the calls in them.
3. `/` picks out one argument.

Press `;` to collapse to a caret and search the whole buffer again.

#### Regular expressions

`/` passes the pattern directly to Rust's `regex` engine. The `/` key opens
the prompt; it is not a delimiter in a JavaScript-style `/pattern/flags`.

| Want | Write | Not |
| --- | --- | --- |
| Case-insensitive | `(?i)hello` | `/hello/i` |
| Case-sensitive | `Hello` (the default) | |
| Scoped flag | `(?i:hello) World` | |
| Across lines | `(?s)foo.*bar` or `foo\nbar` | |

| Inline flag | Meaning |
| --- | --- |
| `i` | Case-insensitive |
| `m` | `^` and `$` match at line boundaries |
| `s` | `.` also matches newline |
| `R` | CRLF-aware multiline boundaries |
| `U` | Swap greedy and lazy repetition |
| `u` | Unicode (on by default) |
| `x` | Verbose mode |

- **Supported:** character classes, alternation, groups, greedy and lazy
  repetition, anchors, word boundaries, Unicode properties, `\d`, `\s`, `\w`.
- **Not supported:** slash-delimited expressions, trailing flags,
  look-around, backreferences.
- Capturing groups compile, but only the complete match is selected.
- Buffer search runs over the whole text, so a pattern can span lines.
  Workspace regex search is line-scoped, because each result names one row, so
  `Space / /` cannot return a multiline match.

`:help regex` has more examples and a compact syntax table.

#### Search keys

`s` and `/` search this buffer. `Space /` widens the same two to the whole
project: `Space / s` mirrors `s` and `Space / /` mirrors `/`. The Finder is
`f` rather than a sigil — `Space f`, its namespace spelling `Space / f`, and
`Space g f` over commits — and `Space / a` and `Space / p` open that Finder
over a wider scope. `Space s` holds selection commands only.

| Key | Action |
| --- | --- |
| `s` | Search the buffer, ignoring case |
| `/` | Search the buffer with a regular expression |
| `n` / `N` | Select only the next / previous match |
| `*` | Select every occurrence of the word or selection under the caret |
| `f` | Find the next typed character in the buffer |
| `Space / s` | Search the workspace, ignoring case |
| `Space / /` | Search the workspace with a regular expression |
| `Space f` or `Space / f` | Open the Finder over files, buffers, and terminals by name or content; `Tab` switches modes and `Ctrl-t` toggles preview |
| `Space / a` | The same Finder over every file, ignore files not consulted |
| `Space / p` | The same unfiltered Finder rooted at a typed path, inside the workspace or outside it |
| `Tab f` in a directory | The same Finder rooted at the directory the explorer shows (`:open-explorer-finder`) |
| `Space s c` | Keep only the primary selection in any multi-selection |
| `Space s e` / `Space s b` | Put a cursor at the end / start of every selected line |
| `Space s a` or `&` | Pad with spaces until every cursor shares the rightmost display column |
| `Space s k` / `Space s r` | Keep / remove selections matching a typed regular expression |

The directory-scoped pickers have no key: `:file-picker-directory` and
`:fuzzy-grep-directory` search below the active file or explorer directory.
The Finder is described under [The Finder](#the-finder).

#### Workspace search

`Space / s` and `Space / /` search every file in the project.

**What is searched:**

- No ignore file is consulted: a gitignored path the Finder omits is still
  searched.
- `.git`, `.runyte`, and `target` are skipped by name, and symlinks are
  skipped.
- `editor.show_hidden_files` applies.
- Files larger than 4 MiB are not read.

**While it runs:**

- Accepting the prompt queues the walk in the background and returns to
  input. The result buffer opens when it completes.
- A newer workspace search supersedes an older one, so a late result cannot
  replace a newer query.
- The status row is temporarily replaced by `Searching workspace`, the query,
  the elapsed time, and a rotating `- \ | /` spinner.

**The result buffer** is one read-only `[workspace search]` buffer, replaced by
each new search.

- Rows are `path:line:column`, a snapshot taken at query time.
- Enter opens the result under the cursor. Movement, selection, copying, `s`,
  `/`, splits, help, buffer switching, and jump history work as usual.
- Visit as many results as you like without rerunning the query; rerun it to
  refresh.

This differs from the Finder's content mode, whose fuzzy query stays live and
so remains a choose-one picker.

### Layout and whitespace display

`Space p` holds text presentation and the commands that lay out selected
lines.

| Key | Action |
| --- | --- |
| `Space p .` | Toggle whitespace markers for this session |
| `Space p s` | Toggle soft wrap for this session |
| `Space p w` | Hard-wrap each selection at `editor.hard_wrap_width` |
| `Space p r` | Reflow selected prose at `editor.hard_wrap_width` |
| `Space p j` | Join selected lines with a typed delimiter |
| `J` | Join selected lines with a space |
| `Space p t` | Align the columns of the selected table |

#### Whitespace markers

`Space p .` shows:

| Character | Marker |
| --- | --- |
| Space | `·` |
| Tab | `→`, followed by enough cells to reach the same tab stop |
| LF or CRLF line ending | one `¬` |

An unterminated final line has no marker. The markers are display-only: text,
offsets, selections, wrapping, and saved files are unchanged. The setting is
`editor.render_whitespace`.

#### Soft wrap

`Space p s` toggles `editor.soft_wrap` for this session.

- Wrapping follows the live pane width, including after a resize. It does not
  use the hard-wrap width and never changes the text.
- It breaks at word boundaries, and within a word only when the word is wider
  than the pane.
- A document whose longest line exceeds 64,000,000 bytes is never wrapped. The
  length is measured once, when the file is read, before anything is drawn.
- Layouts are reused while the text, pane width, and tab width stay the same,
  so moving through a minified file of a few megabytes does not recalculate
  its whole line each keypress. Drawing seeks straight to the visible text,
  even near the end of the line. An edit or a new width recalculates the
  affected layout; the line-length limit above protects against extremely
  costly layouts.

#### Hard wrap and reflow

| | `Space p w` (hard wrap) | `Space p r` (reflow) |
| --- | --- | --- |
| Width | `editor.hard_wrap_width` (80) | `editor.hard_wrap_width` (80) |
| Existing newlines | Kept as boundaries | Refilled; blank lines stay paragraph boundaries |
| Long words | Kept whole unless one word is wider than the line | Kept whole |

**Reflow in Markdown** keeps headings, fenced and indented code, block quotes,
tables, thematic breaks, and separate bullet or numbered items. Wrapped list
continuations get a hanging indent.

**Reflow in source files** changes only `#` and `//` line-comment paragraphs,
repeating the indentation and comment leader on every line, so selecting
nearby code can never join statements. Lists inside comments get the same
hanging indent.

**The scratch buffer** has no path to name a language, so
`editor.scratch_markdown` (on by default) makes `Space p r` refill it as
Markdown.

- Set it to `false` to reflow a scratchpad as plain text.
- Reflow and `?` read the option together: a scratchpad whose lists survive a
  refill is also one that renders. Highlighting, completion, and saving are
  unaffected.
- Scratch text whose first lines already identify a language keeps it, and is
  refilled as a source file.

#### Joining lines

`Space p j` is the inverse of `Space p w`: it removes every line break inside
the selection and asks what to put in their place.

| Typed | Result |
| --- | --- |
| `Space p j Enter` | Lines run together |
| `Space p j Space Enter` | Lines separated by a space |
| `Space p j`, any other text, Enter | That text, such as `, `, ` \| `, or a word, inserted literally |

- Select the lines first: `x` or `X` for whole lines, `v` for part of them.
- Whitespace against each removed break goes with it, so joining indented
  lines leaves no runs of spaces. The first line keeps its indentation.
- Only the selection is joined. A single-line selection reports that it holds
  no line break, and the line after the selection is never drawn in, even
  when a pointer drag ends on it.
- A selected blank line joins as an empty piece and leaves its own delimiter.
- All selections are joined in one transaction, so one undo reverts them.

`J` is the promptless join from Vim and Helix:

- It joins the lines a selection touches with one space, like
  `Space p j Space Enter`.
- A single-line selection or a bare caret pulls the line below up instead.
- A blank line contributes neither text nor a space, so `J` above an empty
  line removes it without a trailing space.
- The file's final line terminator is never joined away, even after `%`.

#### Formatting tables

`Space p t` pads every cell to the widest one in its column:

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

**What counts as a table:** rows opening with `|`, cells divided by `|`, and at
least one separator row of dashes. Without a separator, a selection is refused
even if every line looks like a row. The separator may be anywhere, so a
selection may start on it or end below a footer rule.

**What is kept:**

- A `+---+---+` separator keeps its `+` signs.
- GitHub alignment colons `:---`, `:---:`, and `---:` survive and set left,
  centred, or right alignment.
- An escaped `\|` stays inside its cell.
- A tab inside a cell becomes spaces at `editor.tab_width` stops, because a
  column boundary cannot be worked out from a tab.

**Selecting:**

- Select the rows first. This command widens the selection to whole rows.
- Selections on the same or consecutive rows are formatted together as one
  table.
- Blank lines inside the selection are allowed and left alone, so selecting a
  little more than the table is fine. Rows below the selection are never
  drawn in.
- Rows with different cell counts are squared up with empty cells; no cell is
  dropped.
- All rows take the indentation of the first.
- If any line is neither blank nor a row, or there is no separator, nothing
  changes and an INFO notification says no table was detected.

### Text objects

`m i` selects inside a text object and `m a` around it, as in Helix. Every
object works at every cursor. The result is left in Select mode, so `d`, `c`,
`y`, and `p` act on it and a motion extends it.

| Key | Object |
| --- | --- |
| `m i w` / `m a w` | The word under the cursor / with the space beside it |
| `m i W` / `m a W` | The WORD, everything between whitespace / with the space beside it |
| `m i p` / `m a p` | The paragraph's lines / with the blank lines beside them |
| `m i (` / `m a (`, also `[`, `{`, `<`, `"`, `'`, `` ` `` | Inside / around the enclosing pair; closing brackets are aliases |
| `m i m` / `m a m` | Inside / around the closest enclosing pair of any of those kinds |
| `m i f` / `m a f` | Inside / around the enclosing function |
| `m i t` / `m a t` | Inside / around the enclosing type, such as a class or struct |
| `m i a` / `m a a` | Inside / around the enclosing argument or parameter |

**Examples:**

| Keys | Result |
| --- | --- |
| `m i w c` | Change the word under the cursor |
| `m a ( d` | Delete a parenthesised group with its parentheses |
| `m i p d` | Delete the paragraph's lines |
| `m i " y` | Yank the text inside the quotes |
| `m a f` | Select the enclosing function |

**Words and paragraphs** are read from the text alone, so they work in every
buffer.

- A word is a run of letters, digits, and `_`; a run of punctuation; or a run
  of whitespace. A WORD is everything between whitespace.
- Around adds the whitespace after the word, or before it when nothing follows
  on the line.
- Neither crosses a line break. A cursor on an empty line has no word.
- A paragraph is a run of lines holding text, separated by empty or
  whitespace-only lines. On a blank line, `m i p` selects the run of blank
  lines.
- Around adds the blank lines after the paragraph, or before it at the end of
  the file.
- A paragraph is selected as whole lines, like `x`, so `m i p d` removes its
  lines and `x` extends the selection.

**Delimiter pairs** resolve through the syntax tree when there is one, which
tells a bracket in code from one in a string. Otherwise a balanced text scan
answers: in plain text, in languages without a grammar, while a file is still
parsing, or when the tree finds no pair (as inside a comment).

- Brackets nest and may span lines. The scan reaches 65,536 characters on each
  side of the selection.
- Quotes pair up from the start of the line, so a quoted string spanning lines
  is not found by the scan.
- A backslash-escaped delimiter is text.
- Asking again with a pair selected grows to the next pair out.
- In ordinary Markdown prose, the scan is first bounded to the enclosing
  Markdown syntax node.

**Functions, types, and arguments** need the syntax tree. Without one they say
why and leave the selection alone.

### Structural syntax

| Key | Action |
| --- | --- |
| `Space x e` / `Space x s` | Expand / shrink syntax selection |
| `Space x p` / `Space x c` | Select syntax parent / first child |
| `Space x h` / `Space x l` | Select previous / next syntax sibling |
| `Space x o` | Open the immediate Tree-sitter document outline |
| `Space x x` | Toggle the syntax fold at the cursor |
| `Space x f` / `Space x u` | Fold / unfold all syntax regions in this pane |
| `Space x [ f/c/p` | Go to the previous function / class / parameter |
| `Space x ] f/c/p` | Go to the next function / class / parameter |

- **Bounds.** Expansion keeps Tree-sitter's half-open bounds. Relationship
  commands and the syntax [text objects](#text-objects) show them with the
  block cursor on the last included character, as in Select mode. Yank,
  delete, change, and indent act on exactly the highlighted span.
- **Folds are per pane.** Two panes may collapse different regions of one
  shared buffer. Any edit clears both panes' folds.
- **Folded rows** show an accent-coloured `▸` between the line number and the
  gutter rule, and a muted `… N lines` suffix counting the hidden rows.
- **While parsing.** `Space x` commands and `mm` are dimmed until the syntax
  tree is ready; see [Syntax highlighting](#syntax-highlighting).

### Macros

Macros live in the `Space m` namespace.

| Key | Action |
| --- | --- |
| `Space m m` | Start recording the default macro, or stop the recording |
| `Space m M` then a register | Start recording a macro under that register |
| `Space m r` | Replay the default macro; accepts a count |
| `Space m R` then a register | Replay that macro; accepts a count |
| `Space m l` | List every recorded macro; Enter replays the selected one |

**Recording:**

- `Space m m` starts recording and the same keys stop it. The stopping keys
  are not recorded.
- `Space m m` records into the default macro, register `@`. `Space m M` asks
  for a register first; any printable key names one.
- Only one recording runs at a time. Starting a second is refused rather than
  replacing the first.
- A recording captures raw input in arrival order and replays it through the
  same dispatch, so a replay does exactly what the typing did.

**Example:** add a semicolon to the end of several lines.

1. `Space m m` — start recording.
2. `A ; Escape j` — append `;`, return to Normal, move down.
3. `Space m m` — stop.
4. `Space m r` — replay on the next line. It accepts a count.

**Replaying:**

- Replay runs between frames, not by holding the input loop.
- `Escape` or `Ctrl-c` cancels the remaining work. Inputs already run are kept,
  because a macro may have done things that cannot be rolled back.
- While work remains, the interaction line shows progress and the cancel keys.

**Safety limits** (reaching any of them stops the whole replay):

- Direct and mutual recursion are refused, naming the register chain.
- Distinct macro calls may nest up to 16 levels.
- One top-level replay has a 10,000-unit budget shared by raw key events,
  characters of literal text, counted repetitions, and nested macros. Counted
  commands expand between frames.
- Range operations whose exact meaning cannot be split are limited to 128
  repetitions in one recorded input, and refused before taking effect when
  larger.


## Files and buffers

### File and buffer keys

| Key | Action |
| --- | --- |
| `Space f` or `Space / f` | Open the Finder over files, buffers, and terminals by name or content; `Tab` switches modes and `Ctrl-t` toggles preview |
| `Space / a` | The same Finder over every file, ignore files not consulted |
| `Space / p` | The same unfiltered Finder rooted at a typed path, inside the workspace or outside it |
| `Tab f` in a directory | The same Finder rooted at the directory the explorer shows (`:open-explorer-finder`) |
| `:file-picker-directory` | Fuzzy-find a file or directory below the active file/explorer directory |
| `:fuzzy-grep-directory` | Fuzzy-search contents below the active file/explorer directory |
| `Space / /` | Search the workspace with a regular expression; see [Search](#search) |
| `g f` | Open the selected path or web link, or the complete target under the cursor; relative files are matched beside the active file/explorer and at the project root |
| `Space e` | Open the active buffer's directory as an editable explorer; from a file, select that file |
| `Space E` | Open the working directory (controlled by `:cd`) as an editable explorer |
| `Space d t` / `Space d d` | Toggle the directory tree / reveal the active file in it |
| `Space b b` | Open the buffer list; Enter visits, `Ctrl-t` toggles the preview and `Tab` shows valid actions |
| `Space b c` | Close the active buffer safely (`:close`, `:c`) without changing the pane layout |
| `Space b d` | Compare a fresh immutable disk revision with the active file buffer (`:diff-disk`) |
| `Space b n` | Open a new scratch buffer in the current pane (`:buffer-new`, `:new`) |
| `Space r` | Reload the active text file or refresh the active explorer or supported Git list |
| `Space c y` / `Space c p` / `Space c P` | System clipboard yank / replace the selection, or paste after a bare caret / paste before |
| `?` in a Markdown document or the scratchpad | Render it as formatted text, or return from that page to the source (`:render`) |
| `Ctrl-s`, `:write`, or `:save` | Save |

Explorer keys are listed under [Explorer keys](#explorer-keys), and tree keys
under [Directory tree sidebar](#directory-tree-sidebar).

### Opening files and links

`g f` opens what is under the cursor, or exactly what is selected.

| Target | Opens |
| --- | --- |
| `https://…`, `http://…`, `www.…` | The default browser. `www.` addresses use HTTPS. |
| A file path | The file in the editor. Relative paths are matched beside the active file or explorer and at the project root. Several matches open a picker. |
| A Markdown link or image label | Its destination, including paths in angle brackets, in the source and in rendered `?` pages |
| A link to `#heading` or `file.md#heading` | That heading |

**Inferring the target** with a bare caret:

- Surrounding Markdown wrappers and trailing prose punctuation are left out.
  A path at the end of a sentence ignores trailing punctuation when the
  literal name does not exist.
- An inferred target stays within one buffer line.
- An explicit selection is used exactly.

**In terminal review** (`g f` in NORMAL/review mode):

- It reads the frozen review text, and resolves relative paths against the
  terminal's latest validated directory (or its launch directory) and the
  project root.
- Opening a file leaves the terminal process running. Opening a link keeps the
  terminal view in place.
- A bare caret anywhere on a web link follows the whole URL across rows joined
  by the terminal's automatic wrapping.
- An explicit newline ends a link, unless a program such as Codex or Claude
  Code broke the link inside indented text. The link then continues onto the
  next row when that row is indented to the same text column and the link's
  row reaches the right edge its block wraps at. A link ending before that
  edge, or a row starting at the left edge, is never joined.
- Resizing the live terminal clears wrap information. An already captured
  review keeps its original links.

**Opening by command:** `:open <path>` (aliases `:e`, `:edit`) opens a file or
directory in the active pane; see [Commands](#commands) for path completion.

### Opening binary files

A file whose bytes contain a NUL, or do not decode as UTF-8, is binary.
Opening one asks which program should have it instead of loading it as text.

| Key | Effect |
| --- | --- |
| Up / Down | Choose an offered program |
| Enter | Open the file with it |
| Tab on a remembered choice | Delete it, or make it the default selection |
| Esc | Leave the file alone |

- **Offered programs.** The prompt first selects the desktop's preferred
  application: `xdg-open` on Linux, `open` on macOS, **System default** on
  Windows. Recent explicit choices follow, most recent first, and the most
  recent seeds the prompt.
- **Typing a program.** Type a name on `PATH` or an absolute path. Explicit
  choices are remembered.
- **How it runs.** The program starts detached with no terminal of its own:
  viewers and GUI applications work, but a terminal program cannot take over
  the screen.
- **Detection.** An initial 8 KB probe avoids reading an obvious binary twice.
  The final read still validates the whole file before it becomes a buffer.
- **Where choices are kept.** Runyte's platform cache directory:
  `$XDG_CACHE_HOME/runyte`, else `~/.cache/runyte` on Linux or
  `~/Library/Caches/runyte` on macOS. Without an explicit `XDG_CACHE_HOME` on
  Unix, `~` comes from the effective operating-system account, not the
  inherited `$HOME`, so a privileged launcher cannot put cache files in another
  account's home.

#### Opening binary files on Windows

- Applications open in the background while editing stays responsive.
- An explicit program is remembered only after Windows accepts its launch.
- A launch still unresolved after five seconds reports that uncertainty and is
  not retried. Acceptance does not prove the application displayed the file.
- Viewers can stay open after Runyte exits, subject to restrictions imposed by
  the process that started Runyte.
- An explicit program uses native executable lookup and argument quoting.
  Quote paths with spaces: `"C:\Program Files\Viewer\viewer.exe" --fit`. The
  file is passed as one final argument.
- Shell operators are literal arguments, and script wrappers are not found as
  native programs.
- The system-default handler needs an equivalent ordinary path shorter than
  260 UTF-16 units; an explicitly chosen viewer may support longer extended
  paths.
- Opening from network shares is not part of the validated support.

### Saving and closing

| Command | Effect |
| --- | --- |
| `Ctrl-s`, `:write`, `:w`, `:save` | Save the active buffer |
| `:write <path>` | Save as `<path>` |
| `:write!` (`:w!`, `:save!`) | Save, replacing an existing file or one that changed on disk |
| `:close[!]` or `:c[!]`, `Space b c` | Close the active buffer in place; `!` explicitly discards unsaved text; terminals are refused |
| `:write-buffer-close` (`:wbc`) | Save and close the buffer in place |
| `:window-close` or `:wc` | Close the active pane, but refuse the last pane |
| `:quit[!]` or `:q[!]` | Close the active pane and its uniquely displayed buffer; from the last pane, exit standalone or stop the persistent session and return to a previous running session, with unsaved-change protection |
| `:quit-all[!]` or `:qa[!]` | Exit standalone or stop the persistent session and return to a previous running session regardless of pane count, with unsaved-change protection; never terminate terminals |
| `:quit-here[!]` or `:qh[!]` | Quit and let the shell wrapper change to the active explorer/file directory |
| `:write-quit` (`:wq`) | Save, then close the pane or quit from the last one |

- Closing a buffer keeps every pane. Each pane that showed it returns to its
  own most recently used live buffer, or a scratch buffer when none remains.
- Quitting in persistent mode is described under
  [Quitting and detaching](#quitting-and-detaching).
- Neither `:close[!]` nor any `:quit…` command ends a terminal session; see
  [Managing terminal sessions](#managing-terminal-sessions).

### The Finder

`Space f` opens the Finder: one ranked list of files below the project root,
open buffers, and terminal sessions.

| Mode | Matches | Enter |
| --- | --- | --- |
| Name (default) | Names and paths | Opens the file for editing, a directory in the explorer, or the buffer or terminal |
| Content (`Tab`) | Lines in files, in-memory buffers (including pathless ones), and terminal scrollback plus the current screen | Opens the location; for a terminal, enters review at the matched row |

`Tab` switches modes without clearing the query. `:fuzzy-grep` opens content
mode directly. `:fuzzy-grep-directory` is a separate, files-only content search
rooted at the active file's or explorer's directory.

#### Finder keys

| Key | Action |
| --- | --- |
| Up / Down, `Ctrl-p` / `Ctrl-n`, paging, Home / End | Move |
| `Tab` | Switch name / content mode |
| `Shift-Tab` | Select the previous row |
| `Ctrl-t` | Toggle the preview |
| Enter | Open the selected result |
| `Ctrl-s` / `Ctrl-v` | Open a file result in a horizontal / vertical split (files only) |
| Escape or `Ctrl-c` | Close |
| `Space` with an empty query | Close |
| Backspace / Delete and prompt control keys | Edit the query |

- `q`, `j`, `k`, and other printable letters are query text, not commands.
- After a query has begun, `Space` separates terms. Every filterable list
  follows this rule; a report, which has no filter, always closes on `Space`.
- In other pickers (directory-scoped and fuzzy-content), `Tab` keeps moving to
  the next row and `Shift-Tab` selects the previous one.
- `Space b b` and `Space t t` remain the place for Save, Discard, Close,
  Rename, and other actions. The Finder only opens things.

#### Writing a query

| Query | Finds |
| --- | --- |
| `fpick` | `file_picker.rs` |
| `kmap validate` | `src/keymap/validate.rs` |
| `src picker` | `src/picker.rs` (but `picker src` finds nothing) |
| `docs/` | Directories only, matched without the slash |
| `term build` | Terminals first, then anything else matching `build` |
| `Readme` | Case-sensitive, because of the capital |

- **Subsequence.** A term matches when its characters appear in order.
  Exact basenames, basename prefixes, consecutive characters, and
  path-component boundaries rank highest.
- **Spaces** split the query into terms, which must appear in the typed order.
  Each term is as loose as a lone word, so `ab cd` accepts exactly what `abcd`
  does. What changes is ranking: distance between terms is free, while the
  same distance inside one term is a gap that lowers the score. Typing words
  apart says you expect them apart.
- **Highlighting.** Terms landing on contiguous spans are emphasized as a
  direct match; a term that had to spread out is shown in the secondary
  colour.
- **Smart case.** One capital anywhere makes every term case-sensitive. Both
  pickers and fuzzy grep use this rule.
- **Type hints.** `file`, `terminal`, `term`, and `buffer` move that kind
  first without hiding other matches.

**What name mode searches:**

- Files: absolute, project-relative, `~/`-relative, and basename spellings of
  their paths.
- Buffers: their structural name and path. A file that is open appears once,
  as the live buffer, so unsaved text and naming win over disk.
- Terminals: assigned name, child title, launch program, number, current
  reported directory, and initial directory.

#### Previews

`Ctrl-t` toggles the preview without changing the mode or query.

- A text file: its first 64 KiB, even when the file is larger.
- A directory: a one-level listing of its files and subdirectories.
- A buffer: its in-memory text.
- A terminal: the live screen in name mode; in content mode, a static numbered
  snippet around the matching row.
- A content match: surrounding lines filling the preview's height, resized
  with the overlay, keeping the matching line visible. Up to 512 context lines
  are kept, and the matching line shows its real line number.
- A direct content match (a single word on a contiguous span, or every term of
  a multi-word query on a span of its own) is filled with the primary match
  colour. A match with a gap inside a term fills its individual characters
  with the secondary colour.

#### What is scanned

- Regular files and directories, found natively: no `git`, `find`, `fd`, or
  external fuzzy finder.
- Nested `.gitignore` and `.ignore` rules apply, including ancestor rules when
  `:file-picker-directory` starts below the project root.
- Symlinks are never followed. `.git`, `.runyte`, and the configured workspace
  state directory are never scanned.
- `editor.show_hidden_files` applies.
- Content mode reads non-empty UTF-8 lines of ignore-aware files, shows
  `path:line`, and skips files larger than 4 MiB. Unsaved buffers replace
  their disk contents; pathless buffers and terminal output are searched
  directly.

**Content search runs inside the scan.** Editing the query restarts the walk,
which keeps only matching lines.

- The 50,000-candidate bound that keeps ranking fast limits matches, not how
  much of the project is read, so a match is found wherever it is.
- Typing more after a complete scan narrows what is already loaded.
- `result limit reached` in the title means more than 50,000 matches; a
  longer query resolves it. On a 150,000-line project only a single-character
  query reaches it.

**Responsiveness.** Typing never waits for discovery, ranking, sorting,
resource matching, or previews. The prompt updates on the next frame and a
progress label stays until the rows match the query. Rows from the previous
query may stay visible briefly, so the overlay does not flash empty, but they
cannot be opened until the new ranking arrives. Results for older queries are
discarded.

#### Wider scopes

| Finder | Ignore rules | Root | Title |
| --- | --- | --- | --- |
| `Space f` | Applied | Project root | (none) |
| `Space / a` | Off | Project root | `all files` |
| `Space / p` | Off | A typed path, inside the workspace or not | `all files in <path>` |
| `Tab f` in an explorer | Applied | The explorer's directory | That root's path, when below or outside the project root |

- With ignore rules off, build output, vendored trees, and generated code are
  reachable. Every other exclusion (`.git`, `.runyte`, the state directory,
  symlinks, `editor.show_hidden_files`) still applies.
- `Space / p` completes the path as you type: `~` expands, a relative path
  resolves against the working directory, and `Tab` accepts the selected row.
  Enter also accepts a row while the typed text is not an existing entry, and
  opens the Finder once it is.
- An explorer inside the project keeps the ignore rules from the project root
  down. One elsewhere reads ignore files from its own directory down. Dotfiles
  follow `editor.show_hidden_files`.
- The scope and title survive the `Tab` into content mode, so an ignored
  file's lines are searchable too.

### Buffer list

`Space b b` lists open buffers in the Navigator's columns (see
[Session and destination navigation](#session-and-destination-navigation)).

| Key | Action |
| --- | --- |
| Enter | Visit the buffer, focusing a pane already showing it |
| `Ctrl-t` | Toggle a bounded preview of its in-memory text |
| `Tab` | Actions, starting with **Bring into active pane** |

**Tab actions:**

- **Bring into active pane** shows the buffer here instead of focusing another
  pane.
- **Close hidden buffers** closes clean buffers not visible in any pane,
  keeping unsaved edits and pending saves. It applies to the whole workspace
  even when the list is filtered.
- A modified file offers **Save** and **Discard changes**. Discard asks for a
  separate Enter.
- **Close** appears only once a buffer is clean, never discards edits, and
  redirects every pane that shared the buffer.
- Explorer buffers offer only **Bring into active pane** and **Close hidden
  buffers**. Filesystem changes happen only by editing an explorer and
  confirming its `:write` plan.

**Retention of special buffers.** The eight most recently active clean
generated and special buffers, explorers included, are kept after their last
pane moves away. Activating a ninth retires the least recently used detached
one.

- This keeps `Ctrl-o`/`Ctrl-i` and `Alt-o`/`Alt-i` useful without letting
  `Space b b` or the Finder accumulate stale views.
- A dirty special buffer stays open until saved or discarded.
- An empty, clean scratch buffer retires as soon as its last pane leaves.
  Written scratch text is an ordinary buffer.

### Directory buffers

Open a directory with `runyte <directory>`, `:open <directory>`, `Space e`, or
`Space E`. It becomes an editable buffer: one relative path per line, with `/`
after directories.

```text
docs/
src/
Cargo.toml
README.md
```

#### Editing entries

Edit the listing with ordinary modal editing and multiple selections:

| Edit | Filesystem change |
| --- | --- |
| Change a name | Rename |
| Delete a line | Delete |
| Add a line | Create a file |
| Add a line ending in `/` | Create a directory |
| Add `docs/roadmap.md` | Create a file inside an existing subdirectory |
| Change a path | Move |
| `x` then `y`, go to the destination, `p` | Copy |
| `x` then `d`, go to the destination, `p` | Move (cut) |

- **Copy and cut** work after navigating in the same pane or across split
  panes. The register keeps the filesystem identity, so pasted entries keep
  their contents and directories are copied or moved recursively.
- **A pasted cut** is applied by writing the destination explorer. Writing the
  source first is refused, because that would delete the move's source.
- **Copying within one directory:** rename the pasted row before writing.
- **Nothing happens until you write.** See [Applying changes](#applying-changes).

**Rows that are not changes:**

- Trailing whitespace and whitespace-only rows are ignored. Writing a listing
  with only those edits refreshes it to the canonical view.
- A name ending in whitespace, containing a control character, or not valid
  UTF-8 cannot be told apart from the row syntax, so such a directory refuses
  to open as a listing. New rows are held to the same rule before a
  confirmation opens.
- A directory cannot be moved below itself: editing `dir/` into `dir/child`
  is rejected by `:w` before any confirmation.

#### Applying changes

`:w` or `:write` opens a plan listing every create, rename, move, copy, and
delete.

| Key | Effect |
| --- | --- |
| Enter | Apply, sending deletions to the operating-system trash |
| `P` | Apply with permanent deletion |
| Escape | Cancel |
| Arrows, paging, Home, End | Scroll the plan |

The confirmation shows each reviewed operation and the total.

**Before applying**, the whole plan is rejected if:

- the directory's visible entries changed on disk since it was opened;
- an entry the plan moves, copies, or deletes changed;
- anything changed below a directory the plan moves, copies, or deletes
  (their complete trees are recorded when the plan is prepared).

Activity inside an unaffected child directory does not stale the explorer.

**While applying:**

- On Linux, macOS, and Windows, a move, rename, or copy refuses to overwrite
  an entry that appeared after the plan was checked.
- Creation and rollback need an empty destination name, even when the
  competing entry is a dangling symlink.
- A filesystem without exclusive rename is refused for operations that need
  it; Runyte never falls back to an overwriting rename.
- If an operation fails midway, an ERROR notification names it and every
  operation already applied.

**After applying:**

- Other affected clean explorers refresh, and open files follow confirmed
  renames and moves.
- Affected explorers with unsaved edits are kept, with a warning.
- Deleted open paths are reported as stale, not silently redirected.
- The explorer that applied the plan keeps its edited row order, so a renamed
  or new entry stays where you wrote it. Entering it again later restores the
  sorted listing.

**What is not guaranteed.** These checks prevent destination collisions; they
do not make a plan atomic.

- Earlier operations, including confirmed deletions, may already have
  completed when a later one fails.
- Concurrent replacement of a parent or source entry, changes during
  recursive operations, and native trash path races are outside these
  guarantees.

#### Recovering from a failed plan

If rollback finds that an original name was recreated meanwhile, both entries
are kept.

- The ERROR notification gives the original path and the absolute location of
  the retained original inside a `.runyte-move-…` staging directory. Failed
  copy cleanup can likewise leave a `.runyte-copy-…` directory, also named in
  the notification.
- The notification buffer keeps these details after the interaction line
  moves on, within the notification history and size limits.
- Staging directories can hold the only remaining copy of a file. Runyte does
  not delete them on exit or when another plan runs.
- Inspect both entries before restoring the retained one to a free name, and
  refresh the explorer before preparing another plan.

#### Explorer keys

| Key | Action |
| --- | --- |
| Enter | Open the selected file or directory (the only key that opens; `e` stays the word-end motion) |
| `-` or Backspace | Open the parent directory and select the child just left |
| `r` or `Space r` | Refresh the listing |
| `.` | Show or hide dotfiles |
| `?` | Toggle read-only permissions, owner, group, size, and modification-time columns |
| `x y` / `x d`, then `p` | Copy / cut selected entries, then paste in the destination explorer |
| `:w` or `:write` | Review a filesystem plan before applying edits |
| `Tab` | Actions, including the ones below and the three display settings |
| `Tab t` | Open a terminal in this pane, starting in the shown directory |
| `Tab f` | Open the Finder rooted at the shown directory (`:open-explorer-finder`) |
| `Tab e` | Open the shown directory in the system file manager (`:open-explorer-system`) |
| `Tab s` | Open a persistent session here (see [Session and destination navigation](#session-and-destination-navigation)) |

`Tab t`, `Tab f`, and `Tab e` use the directory the explorer shows, not the row
under the cursor.

- **`Tab t`** keeps the explorer behind the terminal, with any unsaved edits.
- **`Tab e`** uses `xdg-open` on Linux and `open` on macOS. Unapplied edits
  stay in Runyte; the file manager sees the directory as it is on disk.

#### Display settings

| Setting | Toggle | Effect |
| --- | --- | --- |
| `editor.show_hidden_files` | `.` | List dotfiles or leave them out |
| `editor.explorer_details` | `?` | Show detail columns before each entry |
| `editor.explorer_sort` | (`Tab`) | The order rows appear in |

- `Tab` offers all three. They are ordinary settings, so `Space o o` reaches
  them too, and a choice here is written to `config.yaml`: a listing you set
  up once stays that way next session.
- A change re-reads every clean explorer, not only the active pane.
- If the value cannot be saved (no configuration file was loaded, or it uses a
  construct Runyte will not patch), it still applies to this session and the
  status line says it was not written.
- An explorer with unsaved edits refuses a change to dotfiles or order,
  because re-reading would discard them. Details only prefix existing rows, so
  `?` keeps working.

**Details columns** show permissions, owner, group, human-readable size, and
modification time.

- They are presentation only; the file name stays the only editable text.
- Entries from other years show the year instead of the time.
- In a narrow pane, horizontal scrolling moves through the details first, then
  the file name, so no column is permanently clipped.

**Sort orders.** Directories always come first.

| Value | Order |
| --- | --- |
| `name` / `name_descending` | A to Z / Z to A |
| `modified` / `modified_descending` | Oldest / newest first |
| `size` / `size_descending` | Smallest / largest first |

A directory's own size describes how its entries are stored, not how much it
holds, so a size order leaves directories in name order. Entries with equal
keys keep name order, so nothing shifts between redraws.

**Hidden entries and plans.** A listing without its dotfiles is planned
without them: an entry it never showed is not deleted for being absent, nor
reported when it appears. A new name colliding with a hidden one still stops
the plan instead of overwriting it.

**Symlinks** are listed under their own name, followed by a muted
`→ target` hint showing exactly what the link stores.

- The hint is not buffer text: it cannot be selected, edited, or written.
  Every hint in a listing starts in the same column.
- Enter opens what the link points at, so you edit the file Git and the
  language server know about. A broken link says so.
- Rename, copy, cut, and delete act on the link itself.

#### Navigating and splitting

- **One explorer per pane.** Each pane browses with one explorer, retargeted
  as it walks. `Space b b` lists one directory buffer per pane, labelled
  `[explorer] dirname` with its project-relative path (`.` for the project
  root).
- **Remembered rows.** A directory comes back on the row you left. Going to
  the parent with `-` or Backspace selects the child just left, so Enter
  returns to it. If that child is hidden by the dotfile filter, the parent's
  remembered row stays selected.
- **Unsaved edits.** Retargeting discards the listing, so refreshing or
  navigating away from unsaved edits asks first.
- **Splitting an explorer** works like splitting a file: the new pane shows
  the same listing on the same row. Navigate it elsewhere and it gets an
  explorer of its own, which is how copy and cut across two explorers work.
- **Closing an explorer** retires its buffer and keeps every pane. Each pane
  returns to its most recently used live buffer, then any live buffer, or a
  scratch buffer. `:c` refuses unapplied edits; `:c!` drops the plan without
  touching the filesystem.

#### Changes made outside Runyte

An explorer is watched like an ordinary file. When another process adds,
removes, or renames an entry directly in its directory, every pane showing it
gains `[STALE]`.

- The listing is left alone: rows, selections, unsaved renames, and undo
  history survive, since replacing them would discard edits not yet written.
- `Space r` or `:reload` re-reads the directory and clears the marker. So does
  navigating elsewhere or writing a plan.
- Entries hidden by the dotfile filter are not compared, so their changes go
  unnoticed too.

#### Names on Windows

- Reserved device names, alternate data stream syntax, trailing spaces or
  dots, and names differing only in case are rejected.
- Renaming one entry by changing its case is supported.
- Moves between volumes are refused before staging. Copy, check the copy, then
  delete the source.
- Quoted command paths keep literal backslashes, including UNC roots and a
  trailing separator: `:open "C:\work files\note.txt"`.

### Directory tree sidebar

The directory tree is a sidebar at the left of the editor area. It is not a
pane or a buffer.

| Key | Action |
| --- | --- |
| `Space d t` | Show or hide the tree |
| `Space d d` | Show and focus it, expand the active file's ancestors, and select the file |
| `Ctrl-w h` / `Ctrl-w l` | Move focus across the tree boundary (`Ctrl-h` / `Ctrl-l` with `editor.fast_pane_keys`) |
| Escape | Return to the previous pane, keeping the tree shown |

- `Space d d` refreshes those listings, so files created or moved outside the
  editor can be revealed. A file outside the workspace, or a buffer with no
  file, selects the workspace-root row.
- The `[dir tree]` title sits above the root row. The tree stays rooted at the
  workspace root when `:cd` changes the working directory.
- Expansion, selection, scroll position, legend visibility, and a resized
  width survive hiding it.
- Fullscreen and zen cover it temporarily. Terminals narrower than 36 columns
  hide it.

#### Moving in the tree

| Key | Action |
| --- | --- |
| `j` / `k`, Down / Up | Next / previous row |
| Home / End, `gg` / `ge` or `G` | First / last row |
| PageUp / PageDown, `Ctrl-b` / `Ctrl-f` | Page up / down |
| `Ctrl-u` / `Ctrl-d` | Half a page |
| `gt` / `gc` / `gb`, `H` / `M` / `L` | Top / middle / bottom visible row |
| `l` or Right | Expand a directory, or select its first child |
| `h` or Left | Collapse a directory, or select its parent |
| Enter on a directory | Toggle expansion |
| `Space r` | Refresh the selected directory |

Digits choose destination panes (below), not counts.

#### Tree actions

| Key | Action |
| --- | --- |
| `.` | Show or hide dotfiles in the tree. |
| `/` | Search visible entry names with a regular expression. |
| `gw` | Label onscreen entries; type a label to select its entry without opening it. |
| `Ctrl-n` / `Ctrl-p` | Select the next/previous tree search match, wrapping. |
| `n` | Create a file; end its relative path with `/` to create a directory. |
| `d` | Delete the selected entry after `Delete <path>? [y/N]` in the interaction line. |
| `m` | Move to a typed path; Tab completes paths in this prompt. |
| `r` | Rename the selected entry. |
| `v` | Open the selected file in a vertical split. |
| `s` | Open the selected file in a horizontal split. |
| Enter | Open the selected file in an existing pane. |
| `1`–`9` | Open the selected file directly in that numbered pane. |
| Tab | Toggle the dimmed key legend below the horizontal rule. |

**Choosing a pane.** With one pane, Enter, `v`, and `s` act at once. With
several:

1. Pane titles are replaced by numbers from 1, top to bottom and left to
   right. The tree is not numbered.
2. Type a digit to open in that pane, or split it. With more than nine panes,
   type a longer number; Enter accepts a number that is also a prefix of
   another.
3. Escape cancels and restores the titles.

Numbers are recalculated for each request.

**Changing files.** Create, move, and rename apply as soon as the prompt is
submitted; Escape cancels the prompt, and there is no review stage. Delete
needs `y`; Enter, `n`, or Escape cancels. Deletion goes to the trash first.
Every operation rechecks filesystem identities and destination collisions,
refuses to overwrite, and updates affected open buffers. The safety and
non-atomicity limits of [Applying changes](#applying-changes) apply.

**Jump labels** replace the markers before names: one key for nearby entries,
two for farther ones. Only rows visible above the legend are labelled. Escape
or an unmatched key cancels and keeps tree focus.

**The legend** is shown by default and wraps to the tree width: `n: new`,
`d: delete`, `m: move`, `r: rename`, `v: open in v-split`,
`s: open in h-split`, `Tab: legend`, `.: hidden files`, `/: search`. Remapped
keys show their live spellings.

**Hidden files.** The tree starts with `editor.show_hidden_files`. The `.`
override belongs to this workspace's tree and survives hiding it.

- Hiding a selected dotfile, or a hidden ancestor, selects the nearest visible
  ancestor.
- Revealing a file explicitly can still show a hidden path; toggling
  visibility clears that exception.

**Tree search.** `/` opens `tree search (regex):`.

- Enter selects the next matching visible name. Matching ignores case unless
  the pattern uses `(?-i)`.
- Collapsed directories are not searched.
- An empty prompt repeats the previous search, and Escape cancels without
  moving.
- `Ctrl-n` / `Ctrl-p` repeat it; `n` keeps its create action.
- Tree searches do not affect buffer searches.

**Quitting from the tree.** While the tree is focused, `:q`, `:wc`, and
`:close` (and their long and `!` forms) hide it and return focus to the
previous pane, keeping buffers, panes, and terminal sessions. `:qa` quits the
workspace as usual.

#### Width

The preferred width is `editor.directory_tree_width`: 33 columns by default,
12 to 240 allowed. Change it in `Space o o` or the configuration file.

| To | Do |
| --- | --- |
| Resize for this workspace | Drag the tree's right border |
| Widen / narrow by 5 from the tree | `:resize-right + 5` / `:resize-right - 5` |
| Widen the adjacent pane by narrowing the tree | `:resize-left + 5` from that pane |

Other edges behave as for ordinary panes. The tree always leaves at least 24
columns for the pane layout.

#### Refreshing the tree

- Listings load in the background only as directories are expanded.
- Directory symlinks are shown as links and never expanded recursively.
- While the tree is shown, expanded directories are checked every two seconds.
  Entries created, renamed, moved, or deleted outside Runyte appear
  automatically.
- The selected path stays selected while it exists; otherwise the nearest
  visible ancestor is selected.
- Collapsed branches and a hidden sidebar are not scanned. Expanding a branch
  refreshes it, and showing the sidebar resumes checking.

### Files changed outside Runyte

Runyte watches the parent directory of every open file and the directory of
every open explorer, in both modes, even while a persistent TUI is detached.

**When a path no longer agrees with the state accepted at open, save,
reload, or refresh:**

- Every pane showing it, and the status line, show `[STALE]`. The buffer list
  marks hidden stale buffers too.
- `[STALE]` is separate from `[+]`, so an edit that conflicts with disk reads
  `[+] [STALE]`.
- Text, selections, undo, and language-server state are kept.
- The first observation of each disk revision creates one WARNING
  notification.

**What you can do:**

| Command | Effect |
| --- | --- |
| `Space b d` or `:diff-disk` | Open the disk version as `[disk] path [RO]` beside your buffer, in a live side-by-side comparison |
| `Space r` or `:reload` | Reload. A clean buffer reloads at once; a dirty one asks first |
| `:write!` | Save over the changed file |

- **`:diff-disk`** reads the path again. The disk version is on the left and
  your editable buffer on the right; the comparison follows later edits.
  Deleted, unreadable, and binary versions, and versions over the comparison
  size limit, are refused without changing panes.
- **`:reload` on a dirty buffer** names the path and the undo history that will
  be discarded. Escape or `Ctrl-c` keeps the buffer. Enter applies only the
  exact disk revision reviewed; if the file changes again, the buffer is kept
  and the new revision must be reviewed.
- **`:write!`** is the explicit way to replace a changed file. An ordinary save
  refuses a known conflicting revision before save hooks can edit the buffer.
  `[STALE]` clears only after the written file is verified.

**Resolving by itself:**

- When your text and the disk text become equal, Runyte adopts the disk
  version and marks the buffer clean, keeping usable undo history.
- A deleted file stays stale, but an ordinary save may recreate it.
- Binary and unreadable replacements are never loaded into a text buffer.

### Comparing two files

| Command | Effect |
| --- | --- |
| `:diff-this` (`:difft`, `:dt`) | Mark this buffer, or compare it with the one marked before |
| `:diff-this` in the marked buffer | Take the mark back |
| `:diff-off` (`:do`) | Close the comparison |
| `:diff-disk`, `Space b d` | Compare with a fresh disk snapshot |
| `:diff-remote` | Compare a provider document with a fresh remote snapshot |
| `Space g D` | Compare the active file's Git versions; see [Git](#git) |

**Example:**

1. Open `old.rs` and run `:dt`.
2. Open `new.rs` and run `:dt` again.

`old.rs` appears on the left and `new.rs` on the right. If `old.rs` was not on
screen, the second command splits for it. If both were already in panes, those
panes are used.

**Alignment:**

- Corresponding lines sit level. Where one side has lines the other lacks, the
  other side shows a hatched filler row. It belongs to no line: it cannot be
  clicked, labelled by `goto-word`, or moved onto, like the blank area past
  the last line.
- The two sides scroll together. The pane you are in leads and the other
  follows the line facing it, which is not the same line number once the
  files have drifted apart.

**Live changes.** Both files stay editable, and the comparison follows what you
type.

| Shown as | Meaning |
| --- | --- |
| Added | Lines only the right side has |
| Removed | Lines only the left side has |
| Changed | Lines that answer to different lines on the other side |

- Lines replaced by an unequal number of lines are one change, not a deletion
  plus an addition — the same folding the Git gutter does.
- The gutter shows the comparison instead of Git marks while the comparison is
  open.
- Large buffers are compared in the background after an edit. Until it
  catches up, change colours pause and the panes keep the known alignment,
  adjusting for inserted or removed rows. A whole-buffer replacement without
  an edit transaction may align rows by number meanwhile.

**Display:**

- Soft wrap is off while a comparison is open, whatever `editor.soft_wrap`
  says, and returns afterwards: a wrapped line would take different numbers of
  rows on each side.
- Folded regions are expanded for the same reason.

**Ending it.** A comparison needs both panes and both buffers. Closing either
pane, or showing a different buffer in one, ends it. The Git comparison from
`Space g D` also owns the split it created, so closing either side or
`:diff-off` collapses it and restores the previous buffer or the explorer.

### Shell pipes

`:pipe <shell-command>` (or `:| <shell-command>`) replaces each selection with
the command's output.

On Unix:

```text
:pipe sort
:pipe tr a-z A-Z
:pipe jq .
```

| On | Runs | Syntax |
| --- | --- | --- |
| Unix | `/bin/sh -c`, one invocation per selection | Shell syntax: arguments, quotes, and pipelines |
| Windows | Windows PowerShell with no profile, noninteractive, whatever `COMSPEC` says | PowerShell syntax |

**How it works:**

- Each selection is sent on stdin and replaced with stdout. Selected text is
  never inserted into the command, and editor variables are not expanded.
- Programs resolve through the inherited `PATH`.
- The working directory is the workspace root when the command starts.
- Stdout must be UTF-8 and is kept exactly, including trailing newlines. Empty
  stdout deletes the selection.
- All replacements are one undo step. Any failed invocation discards every
  replacement and reports an error.
- Commands can have side effects that undo and cancellation cannot reverse.

**On Windows:**

```text
:pipe [Console]::Write([Console]::In.ReadToEnd().ToUpperInvariant())
```

- That example uppercases each selection.
- Console input, output, and native pipeline output are UTF-8 without a BOM.
- Use `[Console]::In.ReadToEnd()` and `[Console]::Write(...)` when exact
  newlines matter; PowerShell object pipelines format text their own way.
- The workspace directory needs an ordinary Windows spelling shorter than 260
  UTF-16 units.

**Limits:**

| Limit | Value |
| --- | --- |
| Pipe jobs per workspace | 1 |
| Selections per job | 256 |
| Command text | 16 KiB |
| Selected input | 8 MiB |
| Combined stdout | 8 MiB |
| Retained stderr | 16 KiB per invocation |
| Whole-job deadline | 30 seconds |

Selections run one after another.

**While it runs:**

- Editing and rendering continue.
- The result goes to the buffer it came from, even after switching panes. It
  is refused if that buffer changed (including an edit then undo), closed, or
  became read-only. Moving selections does not retarget it.
- `:pipe-cancel` cancels the job.
- The workspace host owns the job in both modes, so detaching and reattaching
  does not restart or cancel it. Host shutdown cancels it.

**Cleanup.** Cancellation, timeout, failure, and completion kill the owned
process tree.

- Unix uses a process group and reaps the shell. Processes that deliberately
  leave the group are outside its cleanup.
- Windows uses a job that forbids breakaway. Inherited output pipes cannot keep
  a finished shell's job alive indefinitely.

There is no default key; the bare `|` key is reserved.

### System clipboard

`Space c y`, `Space c p`, `Space c P`, `Ctrl-v`, and `Alt-v` use the system
clipboard through:

| Platform | Helper |
| --- | --- |
| macOS | `pbcopy` / `pbpaste` |
| Windows | Native Unicode APIs |
| Linux and other Unix | The first available of `wl-clipboard`, `xclip`, or `xsel` |

A missing helper gives an actionable status message and does not affect
Runyte's own registers.


## Panes

### Pane keys

Pane commands live under `Space w`, with `Ctrl-w` as the compatibility
spelling. From Insert and Replace modes, `Ctrl-w` reaches focus movement
(`h/j/k/l`), `w`, `x`, `n`, `p`, and `a`; Terminal Insert adds `v/s` and
`f/z` (see [Terminals](#terminals)).

| Key | Action |
| --- | --- |
| `Space w v` / `Space w s`, or `Ctrl-w v/s` | Vertical/horizontal splits; from a terminal, the new pane opens the working-directory explorer |
| `Space w…` or `Ctrl-w…` | Window focus, close, next, and only-window operations |
| `Space w x` or `Ctrl-w x` | Exchange the active pane's complete content with the previously focused pane, following the content to its new position |
| `Space w =` | Equalize pane widths, then pane heights within each column |
| `Space w f` / `Space w z`, or `Ctrl-w f` / `Ctrl-w z` | Toggle the full-screen pane / the centred Zen viewport |
| `Ctrl-h/j/k/l` | Move between panes without a prefix, when `editor.fast_pane_keys` is on |
| `:vsplit [path]` / `:hsplit [path]` (`:split`) | Create a side-by-side / stacked split |
| `:resize-right +/- N` | Grow or shrink the active pane at its right edge by `N` terminal cells |
| `:resize-left +/- N` | Grow or shrink the active pane at its left edge by `N` terminal cells |
| `:resize-top +/- N` | Grow or shrink the active pane at its top edge by `N` terminal cells |
| `:resize-bottom +/- N` | Grow or shrink the active pane at its bottom edge by `N` terminal cells |

Borders can also be dragged with the mouse. The key-hint popup after
`Space w` lists every window command.

**Examples:**

| Keys | Result |
| --- | --- |
| `Space w v` then `Space f` | Open a file beside the current one |
| `:resize-right + 10` | Make the active pane 10 cells wider |
| `Space w =` | Level every split again |
| `Ctrl-w l` in Insert mode | Move right without leaving Insert mode |

### Splitting

- **A file** splits into two panes showing the same buffer.
- **An explorer** splits the same way: same listing, same row. No key opens the
  entry under the caret in a split. Navigate the new pane elsewhere and it
  gets an explorer of its own.
- **A terminal** is the exception. A terminal is pane content, not a buffer,
  so the pane still holds the buffer it showed before; that buffer is history,
  not something you asked to see twice. The terminal stays where it is, live
  or in review, and the new pane opens the working directory as an explorer,
  as `Space E` would.

### Fast pane keys

Set `editor.fast_pane_keys: true` to move between panes with the bare
`Ctrl-h/j/k/l` — left, down, up, right — as a tmux user would. It works in
Normal, Select, Insert, and a live terminal.

It is off by default because those keys are not free:

| Key | Normally |
| --- | --- |
| `Ctrl-j` | Insert newline (Insert mode) |
| `Ctrl-k` | Delete to end of line (Insert mode) |
| `Ctrl-h` | Backspace, in most terminal programs |
| `Ctrl-l` | Clear the screen, in most terminal programs |

With the setting on, a terminal's child never receives those four keys. Both
prefixed spellings keep working, and help and the key-hint popup describe
whichever set is in force. Configured bindings still decide the action.

On native Windows, Runyte requests keyboard reporting that keeps Ctrl chords
distinct from Backspace and Enter. Fast pane keys also work in the explorer,
where the physical Backspace and Enter keys keep directory navigation and
opening.

### Equalizing

`Space w =` levels splits after `:resize-*` or a dragged border has skewed
them.

1. Every pane gets the same width.
2. Every pane sharing a column gets the same height.

Only the boundaries move; which pane sits beside or above which stays the
same. A pane spanning the full width above a row of others keeps spanning
it, and the row below is levelled.

### Zen and fullscreen

| | `:zen` (`Space w z`) | `:fullscreen` (`Space w f`) |
| --- | --- | --- |
| Maximizes the active pane | Yes | Yes |
| Text width | Centred, up to `editor.zen_width` (100) cells; a narrower terminal uses all cells | Unchanged: laid out exactly as in a split |
| With only one pane | Works, because it changes the text width | Leaves the view unchanged and reports `only one pane` |
| Pane title tag | `[zen]` | `[fullscreen]` |

- The buffer, selections, undo history, and split tree are unchanged. Toggling
  again restores the exact previous layout.
- Soft wrap still follows `editor.soft_wrap`.
- With several panes, the two views share one state: asking for the other
  switches to it, and only the view actually showing toggles off.
- While a pane is maximized, splitting, closing, directional focus, and pane
  cycling wait until it is toggled off. The maximized pane is the only one keys
  can reach, so what is on screen and what receives typing cannot diverge.
- The title tag follows the `[+]` and `[RO]` markers. An ordinary pane has
  neither tag.
- Both cover the directory tree temporarily, and zen hides the session strip.


## Terminals

A terminal session runs a program — a shell, `htop`, `vim`, a coding agent —
inside a pane. It is pane content, not a buffer, and it outlives the pane
showing it.

### Starting a terminal

| Command | Runs |
| --- | --- |
| `:terminal` (`:term`, `:t`), `Space t n`, `Ctrl-w t` | `$SHELL` on Unix, `%COMSPEC%` (else `cmd.exe`) on Windows, in the editor working directory |
| `:terminal htop` | That command line |
| `:terminal-file-directory [command]` | From the active file's parent |
| `:terminal-directory-root [command]` | From the active explorer's root |
| `:terminal-selected-directory [command]` | From the selected directory entry |
| `:terminal-session-directory <number\|name>` | A shell in another terminal's last safe directory |
| `Tab t` in an explorer | A terminal in the directory the explorer shows |

- The pane's own buffer stays where it is; leaving the terminal shows it
  again.
- Unix splits command lines shell-style; Windows uses native
  double-quote/backslash syntax.
- **Directory reports.** Shells may report their directory with a bounded
  local OSC 7 `file:` URL. Remote hosts, control characters, non-file schemes,
  and paths that are not existing absolute directories are rejected.

#### Windows terminals

Windows terminals use ConPTY and need no Bash.

- Executables are found on absolute `PATH` entries using `PATHEXT`. The
  workspace is not searched implicitly.
- Quote paths with spaces:
  `:terminal "C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo`.
- Operators and batch files need an explicit shell:
  `:terminal cmd.exe /d /c "echo hello"`.
- For `cmd.exe /c` or `/k`, give exactly one command-string argument. Runyte
  adds `/s` and keeps that string as shell text, nested quotes included:
  - `:terminal cmd.exe /d /c "echo \"a b\""` prints `"a b"`;
  - `:terminal cmd.exe /d /c "\"C:\scripts\my task.cmd\""` runs a batch
    file whose path has spaces.
- PowerShell 7 and Git Bash are optional installations.
- Each terminal owns an independent process tree; closing it or quitting the
  editor ends that tree.
- Working directories need an ordinary Windows path shorter than 260 UTF-16
  units; paths needing extended-name semantics are refused before launch.
  `cmd.exe` also refuses UNC working directories, so use a shell that
  supports them. These limits do not affect which files the editor can open.

### Terminal modes

| Mode | Keys go to | Shown as |
| --- | --- | --- |
| Insert | The program, except Runyte's few exceptions | `[insert]` in the pane title |
| Live Normal | Runyte; the screen keeps updating | No title marker |
| Normal/review | Runyte; a frozen snapshot you can navigate and copy | Review marker in the title, grayed text |

```text
 INSERT ──Ctrl-\──▶ live NORMAL ──Ctrl-\ or a review command──▶ NORMAL/review
   ▲                    │                                           │
   └──────── i, a, o, or another terminal insert key ◀──────────────┘
```

The title answers whether typing reaches the child, so NORMAL is unmarked: the
mode line already says it.

#### Terminal Insert mode

Every key belongs to the program except:

| Key | Effect |
| --- | --- |
| `Ctrl-\` | Leave to live Normal mode |
| `Ctrl-w` | Start the window prefix |
| `Ctrl-h/j/k/l` | Move between panes, only with `editor.fast_pane_keys` on |
| `Shift-Left` / `Shift-Right` | Visit the previous / next running session, in persistent mode (remappable) |

- `Escape`, `Ctrl-c`, `Ctrl-o`, `Space`, and ordinary keys reach the child
  unchanged.
- **Window keys from a terminal.** `Ctrl-w h/j/k/l` and their control-key
  aliases move to another pane at once. `Ctrl-w v/s` split, leaving the child
  live in the original pane; the new pane opens the working-directory
  explorer, as `Space E` does. `Ctrl-w f/z` toggle full-screen and zen from
  terminal Insert, live Normal, Normal/review, and Select without changing the
  mode. Bare `Ctrl-w`, or cancelling the prefix, keeps the current mode.
  Window actions never capture or discard a review snapshot.
- **Arriving at a pane.** Every focus route — these keys, pane cycling, a
  mouse click — enters a live terminal in Insert mode. A terminal with a
  captured review stays in review until `i`, `a`, `o`, or another insert key.
  A document reached from Terminal Insert starts in Normal.
- **`Ctrl-\` spelling.** On macOS, Runyte asks for unambiguous Ctrl-key reports
  without repeat and release events. A terminal without that protocol sends
  legacy control bytes, where `Ctrl-\` arrives as `Ctrl-4`; both work.

#### Review mode

**Review navigates and copies. It does not edit.** The screen is a picture of
the program's text, and the program owns its own input area, so an edit to
those cells would be an edit to a picture. Commands that would edit are
refused rather than applied to the buffer behind the pane.

A second `Ctrl-\`, or the first review command used from live Normal, captures
the bounded output as an immutable snapshot. New output keeps arriving behind
it.

- Motions move a visible review caret and keep it within the viewport margin.
- `v` enters Select mode; motions then extend inclusively through both the
  start and end characters, in either direction. `Escape` cancels a selection.
- The title marks review and newer output, and the reviewed text is grayed,
  so a frozen image never looks like a live program.
- Moving to another pane does not start review; returning to a reviewed
  terminal keeps its snapshot.
- A live terminal keeps its own colours. Alternate-screen review holds only
  the captured visible screen.

**Pasting back into the program:**

| Key | Sends | Afterwards |
| --- | --- | --- |
| `p` / `P` | Runyte's selected register | Insert mode, so the next key (usually Enter) reaches the child |
| `Space c p` / `Space c P` | The system clipboard | Stays in Normal mode |
| `u` | One delete per character of the last paste | Undoes that paste |

- Both routes leave review first and write at the child's real cursor, never
  into captured output.
- `u` works only while the paste is still the child's last input: any key
  typed into the terminal ends it. It is refused for a paste that ended a line
  the child has already run, since what ran is the child's. A shell at a
  prompt erases exactly the paste.

`i` leaves review and types again.

### Terminal keys

| Key | Action |
| --- | --- |
| `Space t n` or `Ctrl-w t` | Run `$SHELL` in this pane (`:terminal`, `:term`, `:t`; `:terminal <command>` runs something else) |
| `Space t t` | List the running and exited terminals and visit the chosen one (`:terminals`) |
| `Space t r` | Rename this pane's terminal (`:terminal-rename <name>`) |
| `Space t q` | Show this pane's buffer again, leaving the program running |
| `Space t y` | Copy this terminal's output into a read-only buffer (`:terminal-output`) |
| `Space t s` | Send the selection — or the whole buffer — to a terminal as one bracketed paste (`:terminal-send [number\|name]`) |
| `Tab`, then Close in `Space t t` | Explicitly end and forget the selected terminal |
| `Tab`, then Force kill in `Space t t` | Kill the selected terminal's process group and discard its output after confirmation; Enter confirms, Escape cancels |
| `Ctrl-w h/j/k/l` or `Ctrl-w Ctrl-h/j/k/l` in Terminal Insert | Move directly without capturing or discarding review; a live terminal destination starts Insert, a reviewed terminal stays in review, and a document destination starts Normal |
| `Ctrl-h/j/k/l` in Terminal Insert | The exact same destination behavior without the prefix, when `editor.fast_pane_keys` is on; the child stops receiving those four keys |
| `Ctrl-w w` in Terminal Insert | Cycle panes with the same live-terminal/reviewed-terminal/document destination behavior |
| `Ctrl-w v/s` in Terminal Insert | Create a vertical/horizontal split showing the working-directory explorer, while leaving the terminal child live without review |
| `Ctrl-\` in a terminal | First leave INSERT for live NORMAL, then enter review on the second press. Reported as `Ctrl-4` by terminals without the enhanced keyboard protocol, and both work |
| `i` / `a` in a terminal | Type again, returning to the live screen first |
| `h` / `j` / `k` / `l`, word, line, paragraph, and character-find motions | Move the terminal review caret; after `v`, extend an inclusive character selection in either direction |
| `%` in terminal review | Select all retained review text |
| `x` / `X` in terminal review | Select the current line, then extend the moving edge down / up on repeated presses |
| `C` / `Alt-C` in terminal review | Add carets below / above at the same occupied terminal-cell column, skipping short rows |
| `Ctrl-u` / `Ctrl-d`, `Ctrl-b` / `Ctrl-f` | Move the review caret by half / full pages, keeping it visible |
| `gg` / `ge` in a terminal | Move to the oldest / newest rows in the captured review snapshot |
| `gf` in terminal review | Open the selected file path or web link, or the target under the caret; inferred paths at the end of a sentence ignore trailing punctuation if the literal name does not exist. Web links use the default browser, including links broken across rows by the terminal or by an agent's indented output |
| `gw` in terminal review | Label visible terminal words and jump to the chosen one |
| `f` / `F` / `t` / `T` in terminal review | Find characters in the snapshot |
| `s` / `/`, then `n` / `N` | Search an immutable terminal review snapshot by literal / regular expression and move among matches |
| `)` / `(` in terminal review | Move through search matches, like `n` / `N` |
| `y` / `Space c y` in terminal review | Copy the caret character or every selection, joined by newlines, to the unnamed register / system clipboard |
| `p` / `P` in a terminal | Leave review and send Runyte's selected register to the live program |
| `Space c p` / `Space c P` in a terminal | Leave review and send the system clipboard to the live program |
| `u` in terminal review | Take back the last paste, while it is still the child's last input |

Review search is case-insensitive for `s` and a regular expression for `/`,
with stable highlighted matches.

### Moving text in and out

**Out of a terminal.** `Space t y` (`:terminal-output`) copies the session into
an ordinary read-only buffer. That is real text: search, multiple selections,
`n`/`N`, and yank all work. `Ctrl-o` or `Alt-o` returns to the terminal, and
`Ctrl-i` or `Alt-i` goes back to the output.

**Into a terminal.** Write the text in an ordinary buffer, with every editing
command available — multiple cursors above all — then `Space t s`
(`:terminal-send [number|name]`) sends the selection to a terminal as one
bracketed paste, or the whole buffer when nothing is selected.

This is the only way modal editing can reach a program that owns its input
area, and it is what makes long prompts for a coding agent worth writing in the
editor.

**Example:** write a prompt for an agent running in terminal 2.

1. `Space b n` — open a scratch buffer, then `i` and write the prompt.
2. Escape — back to Normal mode, with nothing selected.
3. `:terminal-send 2` — paste the whole buffer into terminal 2.
4. Switch to the terminal and press Enter there to submit.

### Managing terminal sessions

A session outlives the pane showing it.

- `Space t q` shows the pane's buffer again and leaves the program running.
  Opening a file in the pane, or closing the split, does the same.
- When the program exits, a pane showing it returns to its most recently used
  buffer (or a scratch buffer) without closing. The exited session and its
  bounded output stay in `:terminals` for review, search, or Close.

#### The terminal list

`Space t t` (`:terminals`) lists sessions in the Navigator's columns (see
[Session and destination navigation](#session-and-destination-navigation)):

- running sessions first, then exited ones, dimmed; each group most recently
  activated first;
- `exited`, `unread`, and `bell` in the STATE column;
- the program, number, and directory in the preview.

**The preview** shows the live screen with its colours, updating while you move
through the list; an exited terminal shows its final screen. It is read-only
and never resizes the terminal. A wider screen is clipped, and a shorter
preview keeps the child's cursor row visible. `Ctrl-t` toggles it.

| Action | Effect |
| --- | --- |
| Enter | Visit: focus a pane already showing it, or show it here. An exited session opens in review. |
| **Show** (Tab) | Show the session in the active pane. The only action that attaches one. |
| **Bring into active pane** (Tab) | Move a session shown elsewhere to this pane; the old pane shows its buffer again. One PTY is never resized by two visible panes. |
| **Rename** (Tab) | Ask for a name and return to the list, leaving panes unchanged |
| **Close** (Tab) | End and forget the session. A hidden live process needs a second Enter. For an exited session, removes its output. |
| **Force kill** (Tab) | For a hung program. Always asks, naming the terminal, even when visible: Enter confirms, Escape cancels. Kills the process group, discards its output, and shows the pane's buffer. Unsaved work in the program is lost. Not offered for exited terminals. |
| **Create** (Tab) | Start a new terminal |

**Ending a program is always explicit:** type `exit` in it, or choose Close or
Force kill. Neither `:close[!]` nor any `:quit…` command ends a terminal, and
every quit spelling, `!` included, refuses while any terminal is running,
pointing to `:terminals`.

**Other commands:**

- `Space t r` (`:terminal-rename <name>`) names the active session.
- `:terminal-show <number|name>` shows a specific session.
- Duplicate names are refused as ambiguous.

#### Numbers and titles

A pane title reads `[terminal #<number>] <name>`.

- The number is what `:terminal-send`, `:terminal-show`, and
  `:terminal-session-directory` accept. It is also in the list preview.
- Only running terminals have numbers. A new terminal takes the lowest free
  number, and gives it up when its program exits or it is closed, so numbers
  stay small and are reused. Numbers never depend on list order.
- Because a number can pass to a new terminal, it identifies what is running
  now. Do not record one in a macro or script expecting the same terminal
  later.
- An exited terminal has no number: its title is `[terminal] <name>`, and it
  is reached from `:terminals` or the Finder by name. Action menu titles use
  the `[terminal] <name>` form too.
- The name is the one you assigned, otherwise what the program calls itself:
  the title a shell sets from its prompt, or the program's name until it sets
  one.
- Scrolled back into history, the title adds `↑` and how far.
- The active pane adds `[insert]` while keys go to the child.

#### Lifetime

Sessions live as long as the workspace-host process that owns them:

| Survives | Does not survive |
| --- | --- |
| Normal detach | Force-stop |
| Client failure | Host replacement or crash |
| Switching workspaces | Logout or reboot |
| Reattaching | Machine failure |

`:detach` leaves persistent terminal children running without signalling them.
In standalone and persistent modes alike, quitting refuses while a terminal
runs.

### Emulation and limits

The emulator is Runyte's own, like the fuzzy scorer, the picker, and the diff.

**Supported:** colour including the 256-colour palette and true colour, the
usual attributes, scroll regions, insert and delete, the alternate screen,
bracketed paste, application cursor keys, cursor-position reports, and window
titles.

**Scrollback and performance:**

- Scrollback is bounded at 5,000 lines per session and by a measured 64 MiB
  retained-cell and review budget per workspace.
- Noisy sessions use independent bounded queues and round-robin byte and
  message budgets. PTY input is bounded and chunked.
- Terminal and review output is never written to disk.
- Output frames are coalesced to a bounded cadence. Repainting sends only
  changed rows, with a full resynchronization when the base is stale.
- Rapid identical wheel reports are merged into bounded requests, keeping
  their scroll distance and their order relative to clicks and keys.
- The alternate screen has no history, so while `htop` or `vim` runs there is
  nothing behind the screen to scroll to or copy.
- Inline TUIs on the primary screen may keep a composer or status area fixed
  while completed output scrolls through a top-anchored region. Those
  completed rows are ordinary scrollback, including Codex output in inline
  mode.

**Mouse.** SGR mouse reports are forwarded inside the pane body when the child
asks for them. Borders stay Runyte's, and the wheel scrolls review history
when the child has not asked for the pointer.

**Colour queries.** Read-only `OSC 10;?` and `OSC 11;?` queries get the theme's
default foreground and background, so light- and dark-aware programs pick
fitting colours. A theme colour set to `reset` is unknown and gets no reply.

**Known limitations:**

- Colour-setting and palette queries are ignored, as is `OSC 52`, so a program
  cannot change the palette or write your clipboard unasked.
- A cell keeps up to three combining marks without taking columns; more are
  deliberately dropped.
- Inline images — kitty graphics, sixel — are not passed through.
- Resizing does not reflow wrapped lines. Emulators disagree about what a
  resized wrapped line should become, and a wrong guess corrupts a live
  full-screen program worse than truncation does.
- Windows provides standalone and persistent ConPTY terminals.


## Git

Git support needs `git` on `PATH` and a project inside a Git working tree.
Otherwise there is simply no gutter and no branch in the status line, and the
`Space g` namespace is dimmed.

### Git keys

| Key | Action |
| --- | --- |
| `Space g g` | Open the changed-file list |
| `Space g d` | Open the active file's unstaged diff |
| `Space g D` | Compare the active file's complete Git versions side by side |
| `Space g b` | List local and cached remote branches and check one out locally |
| `Space g w` | Open the repository worktree list |
| `Space g l` | Open paged commit history |
| `Space g f` | Fuzzy-search commits in a hash/title list with author, date, and full-message preview |
| `Space g B` | Open live-buffer attribution for the whole file |
| `Space g t` | Open the bounded stash list |
| `Space g r` | Re-read branch, changed files, and changed lines from Git |

Every Git view uses `Tab` for its actions; see
[Contextual actions](#contextual-actions-tab). The `:git-…` commands are listed
under [Commands](#commands).

**A typical commit:**

1. `Space g g` — open the changed-file list.
2. Select rows, then `Tab s` — stage them.
3. `Tab c` — write the message in the buffer that opens.
4. `:w` — commit.
5. `Tab P` — push.

### Status line and gutter

**Status line.** `main ↑1 +2 ~3 -1 ?4 !1` reads as: branch `main`, one commit
ahead of upstream, two added, three modified, one deleted, four untracked, and
one conflicted file. Each file is counted once, under the most consequential
thing that happened to it.

**Gutter.** Tracked files get a change mark between the line number and its
separator:

| Mark | Meaning |
| --- | --- |
| `+` | Added line |
| `~` | Modified line |
| `-` | Lines were removed here (on the surviving row that closed the gap) |

- Marks compare against the **index**, not `HEAD`, so staging a change removes
  its marks.
- A mark appears only on a logical line's first screen row; soft-wrap
  continuations use the cell for their `↪` arrow.
- Fold triangles share the cell. Only when a folded line is itself changed
  does the gutter add a second cell so both `▸` and the mark show.
- Files with no unconflicted index entry — untracked, mid-merge, or binary —
  show no change column, rather than one claiming every line is new.

**How marks stay current.** Git is asked for a file's staged text once, when
it opens and after relevant Git changes while it is visible. Everything else
is diffed in memory, so marks follow typing without a process per keystroke.

- Large files are compared on a background worker: a changed gutter clears
  briefly, then shows marks for the newest completed revision.
- Hidden buffers reload their staged text only when a pane shows their gutter.

### Automatic refresh

Filesystem changes to the worktree, index, `HEAD`, refs, packed refs, stashes,
and linked-worktree metadata trigger one debounced background refresh.
`git.refresh_interval_seconds` bounds how often (see
[Automatic Git refresh](#automatic-git-refresh)). `:git-refresh` or
`Space g r` reconciles immediately.

**A refresh waits while you work**, because it rewrites a Git view's text and
moves the cursor. It is held back:

- until you pause briefly;
- while a prompt is open, including `s` and `/` queries;
- while a Git view holds a deliberate selection, such as the matches `s`
  leaves.

Nothing is dropped: the refresh runs as soon as you stop, the prompt closes,
or the selection collapses. A selection in an ordinary file defers nothing,
because a refresh changes its gutter, not its text.

**The cursor keeps its place.** Its row is matched by identity — the commit,
path, branch, or hunk it was on — and falls back to the nearest surviving
row. The column is clamped to the end of whatever row it lands on.

### Changed files, staging, and committing

`Space g g` opens the changed-file list: files grouped by whether a commit
would take them, one per line. A selection over several rows is a selection of
files, and one key acts on all of them.

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

| Key | Action in the list |
| --- | --- |
| `Tab s` / `Tab u` | Stage / unstage every file the selection covers |
| `Tab S` | Stage every unstaged and untracked file in the list |
| `Enter` | Show this row's diff |
| `Tab d` | Compare this row's complete versions in a temporary split |
| `Tab o` | Open the file on this line |
| `Tab D` | Discard the selected files' changes |
| `Tab c` | Write a message and commit what is staged |
| `Tab i` | Review everything staged for the next commit |
| `Tab p` / `Tab P` | Pull / push the branch this working tree is on |
| `Space g r` | Re-read everything from Git |

#### Reading the changed-file list

**The first row** names the branch, its drift from upstream, and how many
files are in each section. It counts sections rather than per-file states, so
it never calls a file "modified" above a heading that calls it staged. The
status line's compact `~1` answers a different question: what changed, rather
than where it sits.

**Line counts** show what each change costs, added then removed, in one
aligned column; the first row totals them.

- They use the theme's `change_added` and `change_removed`, the same colours
  as the gutter and diffs. Built-in themes use green and red; custom themes
  can change either.
- They are Git's own `--numstat` counts of the same two trees the row comes
  from. A file staged and then edited again is counted once on each of its
  rows, and the total is the sum of what is shown.
- An untracked file counts every line as added.
- A change that cannot be counted — a binary file, an untracked symlink, a
  file over a megabyte, or a whole untracked directory Git collapsed into one
  row — shows `·` in both columns and is left out of the total. A list with no
  countable rows omits the columns.
- Counts are read only while the list is open.

**A file in two sections** (staged, then edited again) is not a duplicate:
those are two changes, staged separately. `Enter` on a staged row shows what a
commit would take; on an unstaged row, what it would not. A selection covering
both acts on the file once.

**After staging**, the caret follows the file into its new section, so `Tab u`
undoes what you just did.

#### Staging

- `Tab s` records each file **as written on disk**. When a buffer has unsaved
  changes, an INFO notification says so.
- Staging moves the gutter's base, so marks for staged lines disappear at
  once.
- For a rename, the displayed destination is what you open or diff, while
  staging and unstaging act on both paths, so a move is never split across the
  index.
- The changed-file list reads files on disk, not buffers — the one place the
  gutter and the diff views can disagree. The diff header says so when a
  buffer has unsaved changes.

**Hunks.** In a per-file diff, `Tab s` stages the exact hunk under the cursor
and `Tab u` unstages the exact staged hunk. The request carries the hunk bytes
plus repository, HEAD, index, file, and live-buffer preconditions; Git checks
the patch, and any stale precondition changes nothing.

**Lines.** `:git-stage-lines` stages a deliberately narrow slice from a saved,
clean source buffer: one contiguous selection containing every added or
modified new line in one hunk.

- Refused: dirty buffers, deletion-only choices, multiple or partial hunks,
  binary files, conflicts, renames, and untracked files.
- A refused partial action never becomes whole-file staging.
- Use Lazygit for finer patch surgery, conflict resolution, or advanced
  history work.

#### Committing

`Tab c` opens a commit message buffer with the template Git would give an
external editor: an empty first line, then commented instructions and the
files to be recorded.

| To | Do |
| --- | --- |
| Commit | Write the buffer: `:w` or `:wq` (`:wq` does not exit Runyte here) |
| Cancel an unchanged message | `:c` |
| Cancel an edited message | `:c!` |

- The index is refreshed first, so staging done outside the editor is
  included.
- A commit takes **the index** — exactly what the Staged section shows.
- Comment lines are not part of the message, so a message line cannot start
  with `#`.
- Cancelling commits nothing and leaves the index as it was.
- A refused commit (an unset identity, a rejecting hook) keeps the message in
  the buffer to fix and retry.
- Afterwards the pane returns to where you started.
- Hooks run in the background commit, so a slow `pre-commit` leaves the editor
  responsive. A signing key that wants a terminal prompt fails rather than
  asking.

#### Discarding

`Tab D` restores the selected files to `HEAD`, throwing away both staged and
unstaged changes.

- It is the only Git action here that cannot be undone: the content was never
  a commit, so no reflog brings it back. It asks first and names what it will
  take.
- A file with unwritten buffer edits is refused, so Git never overwrites text
  that exists only in the editor. Clean buffers reload afterwards.
- Discarding a staged addition removes its clean open buffer with the file.
  Discarding a rename restores the original path and removes the destination.
- Untracked files are refused. Discarding one could only mean deleting it,
  which Git keeps behind `clean` and Runyte keeps in the explorer, where
  deletion is a confirmed plan that goes to the trash.

### Diffs

| Key | Opens |
| --- | --- |
| `Space g d` | The active file's unstaged patch |
| `Space g D` | Two complete, aligned versions: index on the left, working tree on the right |
| `Tab i` | Everything staged for the next commit |

- In the changed-file list, `Space g d` and `Space g D` follow the selected
  row, so a staged row compares `HEAD` with the index.
- A missing side of an added or removed file is shown as hatched filler.
- Patches are coloured by line: added and removed lines in the gutter colours,
  hunk positions in the accent colour, headings muted. The same colouring
  applies to all generated patch views.

**The side-by-side pair is temporary.** Closing either pane removes the split
and returns the surviving pane to the buffer active before the comparison.
`:diff-off` (`:do`) does the same from either side. If that buffer has since
been closed, the pane opens the workspace explorer.

### Branches

`Space g b` opens one read-only list, Local first and Remote second:

```text
Local
  feature [↑1] [worktree: /home/me/project-feature]
* main    [↑2 ↓1] [worktree: /home/me/project]
  spike

Remote
  origin/feature   [tracked by: feature]
  origin/main      [tracked by: main]
  origin/review/42 [not tracked locally]
```

| Key | Action in the list |
| --- | --- |
| `Enter` | Check out this local or remote branch locally |
| `Tab n` | Start a new branch here and switch to it |
| `Tab w` | Create a worktree for this branch; attach in persistent mode |
| `Tab d` | Compare committed tips with this branch |
| `Tab D` | Delete this local branch, with its worktree and session, after a confirmation |
| `Tab f` | Fetch this cached remote branch or this local branch's upstream |
| `Tab p` | Fast-forward the current local branch onto what it tracks |
| `Tab P` | Publish this local branch to what it tracks |

#### Reading the branch list

- `*` marks the current local branch.
- `[worktree: ...]` names the local path of each registered worktree that has
  the branch checked out.
- **Drift** is shown in a dimmed column of its own:

  | Marker | Meaning |
  | --- | --- |
  | `↑N` | N commits the local branch has that its upstream lacks |
  | `↓N` | N commits the upstream has that the local branch lacks |
  | `[=]` | In step |
  | `[gone]` | The upstream ref no longer exists |
  | (nothing) | The branch tracks nothing |

- **Remote rows** are remote-tracking refs Git has already cached; opening the
  list and `Space g r` do not fetch.
- A symbolic default such as `origin/HEAD` is not a row. Remote names
  containing `/` are kept exact.
- Each remote row names every local branch whose configured upstream is that
  exact ref. Equal names or equal tips do not count as tracking; a remote row
  with no tracker says `[not tracked locally]`.

#### Checking out

- **A local row:** Enter checks it out.
- **A remote row:** Enter checks out its one local tracking branch. With
  several, a picker opens. With none, Runyte creates and checks out a tracking
  branch named after the part after the remote: `origin/feature/auth`
  suggests `feature/auth`. If that name already exists, an editable name
  prompt opens instead of reusing the unrelated branch.

**Refusals and confirmation:**

- A checkout (and `Tab n`) is refused while the index or working tree has
  staged, unstaged, or untracked changes, or an open file buffer in the
  repository has unsaved edits.
- If this workspace owns any live terminal session — visible, hidden, or in
  another pane — Runyte asks you to type the exact target branch name. That
  acknowledges the terminal job keeps its directory while Git replaces files
  under it. Escape or `Ctrl-c` leaves the checkout unchanged. Exited terminals
  need no confirmation.
- After a checkout, open files that still exist are reloaded from the new
  branch, and the Git status and gutter bases are refreshed.
- Opening another worktree with `Space g w` is an attachment switch, not a
  checkout, and does not ask.

#### Creating and deleting branches

**`Tab n`** asks for a name, creates the branch at the selected one, and
switches to it. The checkout refusals are reported before the name is asked
for.

**`Tab D`** first reviews the exact branch tip.

- If the tip is kept by the configured upstream or another local branch, Enter
  confirms, and the confirmation names those branches.
- Otherwise you must type the exact branch name.
- Upstream reachability uses the locally cached remote-tracking ref, and the
  confirmation says so. Fetch first if it must include newer remote state.
- The final step rechecks the tip and the refs keeping it, so a branch changed
  after review is not deleted.

**A branch checked out in a worktree** is not refused. A worktree means nothing
without its branch, and a session means nothing without its worktree, so
deleting the branch offers to remove all three. The confirmation names every
level and always requires the exact branch name:

```text
Delete branch enh/render-space.
This also:
  · stops and forgets session 5 (runyte-enh-render-space)
  · removes worktree /home/me/code/runyte-enh-render-space
Type enh/render-space exactly to continue.
Escape keeps it.
```

- It runs bottom-up: the session stops, the worktree is removed, then the
  branch is deleted. A failure at any level stops there.
- The checkout's own refusals — a dirty working tree, a lock, a session with
  unsaved buffers — are reported before you are asked anything.
- A branch checked out in more than one worktree, or at the current Runyte
  root, is still refused before review.

#### Creating a worktree from a branch

`Tab w` asks for a destination and creates a checkout of the selected local
branch.

- A branch already checked out here or in another worktree is refused without
  forcing a duplicate; the message names the existing path and points to
  `Space g w`.
- On an untracked remote row, it creates the suggested tracking branch
  directly in the new worktree. A remote with several trackers asks which to
  use.
- In persistent mode the new worktree is attached at once. In standalone mode
  you stay in the current workspace.

### Worktrees

`Space g w` (`:git-worktrees`) lists every checkout registered with the
repository: linked, detached, locked, prunable, and bare.

| Key | Action in the worktree list |
| --- | --- |
| `Enter` | Attach to this root's persistent session, starting it if necessary |
| `Tab n` | Name a new branch at this checkout's tip and create its worktree; attach in persistent mode |
| `Tab d` | Compare committed tips with this worktree |
| `Tab D` | Remove this worktree and its session after confirmation; keep its branch |
| `Space g r` | Re-read the registered worktrees |

- `*` marks the current root. Unavailable states are written on their rows.
- Paths are the identity, including paths that are not valid UTF-8 (displayed
  with replacement characters).

**Opening another root** never retargets this workspace's buffers or language
servers.

| Mode | Enter | `Tab n` |
| --- | --- | --- |
| Persistent | Detaches the TUI and attaches to the destination's host (starting it if needed). The old host keeps its buffers and terminals. | Creates the worktree and attaches at once |
| Standalone | Explains that attaching needs `workspace.mode: persistent` | Creates the worktree and stays in the current workspace |

#### Removing a worktree

`Tab D` removes one ordinary worktree at a time and never deletes its branch.

**Refused before confirmation:**

- staged, unstaged, or untracked files;
- a running session on it with unsaved file buffers or unavailable health;
- the current Runyte root, and locked, bare, missing, or otherwise unavailable
  worktrees.

**What the confirmation asks for:**

| Situation | Confirm with |
| --- | --- |
| A clean session runs on it | Typed text — the branch name, or the displayed path for a detached checkout (the session goes too) |
| Its branch has commits ahead of the cached upstream, or the upstream is gone | The exact branch name |
| An unretained detached checkout | Its displayed path |
| Otherwise | Enter |

```text
Remove worktree /home/me/code/runyte-enh-render-space.
This also stops and forgets session 5 (runyte-enh-render-space).
Branch enh/render-space will remain.
Type enh/render-space exactly to continue.
Escape keeps it.
```

**Order of events:**

1. Git status, worktree identity, upstream state, and session health are
   checked again.
2. The session is stopped — the host owns the directory and its runtime state.
   A failed stop leaves the worktree standing.
3. Git removes the directory. If Git refuses, the branch and history record are
   left alone.
4. The workspace's history record is forgotten, so nothing keeps a claim on
   the digit it used. This happens whether or not a host was running.

Stopping the session is part of removal, not a `session` command, so it
happens in standalone mode too: a standalone editor cannot attach to a
session, but it can still find and stop one running on the worktree.

### Committed comparisons

`Tab d` in the branch or worktree list opens a read-only **committed
comparison**: the current tip on the left, the selected local branch, cached
remote branch, or worktree HEAD on the right.

- It compares the contents at the two tips directly. Uncommitted changes,
  staged changes, and unsaved buffers are excluded.
- Nothing is fetched or checked out.
- Detached worktrees are labelled with their commit ID. A worktree with no
  available commit cannot be compared.

| Key | Action in the committed comparison list |
| --- | --- |
| `Enter` | Open this file's unified patch |
| `Tab d` | Open this file's complete versions in a temporary split |
| `Space g r` | Capture fresh tips and refresh the list |

**Reading it:**

- The heading names both sides and their captured commit IDs.
- Each row shows the path on each side and added/removed line counts. The
  summary totals and per-file counts use `change_added` and `change_removed`
  (green and red in built-in themes); only the signed counts are coloured.
- Renames show both names; `—` marks an absent side. Narrow panes combine the
  two paths into one column.
- Binary and metadata-only changes are labelled. Empty added or deleted files
  stay visible.
- Identical tips show `No committed differences.`

**Stability:**

- The list, counts, patches, and splits all use the captured commits, even if
  a branch moves while you browse.
- Refresh keeps the selected file when possible.
- Closing a patch returns to the list at the same file and scroll position.
  Closing either split side, or `:diff-off`, collapses the pair and restores
  that position too.
- Movement, search, selection, and copying work as usual. Binary files show
  their patch metadata but cannot open as a text split.

### History, search, and blame

#### Commit history

`Space g l` opens history in pages of up to 10,000 commits, newest first in
Git's topological order.

| Key | Action |
| --- | --- |
| Enter | Open the commit's metadata and patch |
| `Ctrl-n` / `Ctrl-p` | Next / previous page |
| `Space g r` | Re-read the view |

- **First line:** current and total pages, the earliest and latest author
  dates, and a paging reminder, separated by `|` and within 80 characters.
  The reminder is a muted hint, not buffer text: it cannot be selected,
  searched, or copied.
- **Each row:** short object ID, author date and time as `YYYY-MM-DD HH:MM` in
  the commit's timezone, author, and subject. The full ID is kept behind it.
- **Refs:** a commit's branches and tags are a muted hint one space past that
  row's own text, not aligned to a shared column, so a long subject never
  pushes a shorter row's hint off a narrow pane.
- **Paging** is on Ctrl chords so `l` and every other motion keep working.
  Pages use boundary objects instead of an unbounded result, so going back
  re-requests the earlier page.
- **Refresh** keeps the caret on the same commit when it is still on the page,
  even if new commits appeared above; otherwise the nearest row. Only the
  first page refreshes automatically; later pages sit behind a commit boundary
  and cannot change.

#### Searching commits

`Space g f` opens a fuzzy picker over commits reachable from `HEAD`, newest
first.

- It matches subjects, message bodies, the object ID (a prefix is enough),
  the author, and the author date.
- Rows rank as lines, not paths, so `/` is an ordinary character.
- Spaces split the query into terms that must appear in order. The first
  `Space` in an empty filter still closes the picker.
- Rows show the subject, author date, author, and abbreviated ID. The preview
  shows the full message.
- Enter opens the same commit detail as the log and blame.
- Discovery is asynchronous and capped at the newest 5,000 commits; the picker
  says when the cap is reached.

#### Blame

| Command | Shows |
| --- | --- |
| `:git-blame` | Attribution for the primary line, without leaving the file |
| `Space g B` (`:git-blame-file`) | A read-only view aligned to the source rows; Enter opens the row's commit |

- Both send the current in-memory text to Git's porcelain blame, so an unsaved
  line says `uncommitted` instead of borrowing an older attribution.
- A result is discarded if the buffer changes while Git works.
- Inputs over 4 MiB, and whole-file views over 20,000 lines, are refused.
  Untracked or otherwise unblamable files report Git's refusal.
- The full-file view also shows each commit's author date as `YYYY-MM-DD`, in
  the commit's timezone.

### Stashes

`Space g t` opens a bounded, read-only stash list. Rows keep full stash object
identities.

| Key | Action |
| --- | --- |
| `Tab a` | Apply the selected stash, keeping it |
| `Tab D` | Drop it, after confirmation |
| `Space g r` | Refresh, keeping the selected stash |

Creating a stash uses a command that names its scope:

| Command | Effect |
| --- | --- |
| `:git-stash-tracked <name>` | Records the tracked worktree and index, leaving staged changes applied |
| `:git-stash-all <name>` | Records the same tracked state, and clears both worktree and index |
| `:git-stash-untracked <name>` | Additionally includes untracked files |

- Every create, apply, and drop asks for confirmation.
- Create and apply are refused while the repository has unsaved editor
  buffers.
- An apply conflict keeps the stash and reports that resolving it belongs in
  an external Git tool.

### Fetch, pull, and push

`Tab f` in the branch list fetches one branch. Pull and push use `Tab p` and
`Tab P` in the branch list and the changed-file list. `:git-refresh` re-reads
what is already local.

#### Fetch (`Tab f`)

- A remote row fetches its exact server branch into the selected remote-tracking
  ref. A local row fetches its configured upstream, including a differently
  named or currently missing upstream.
- Configure a remote upstream first for a local branch that has none. A local
  `.` upstream is not a network target. Excluded or ambiguous fetch mappings,
  symbolic destinations, and mappings outside `refs/remotes/` are refused.
- Only the selected remote-tracking ref changes. Other cached branches and tags
  stay in place; no pruning or tag fetching occurs. A force-pushed upstream may
  replace that cached ref, and the retained Git operation notification identifies
  the forced update.
- Local branches, the index, and working files remain intact. Unsaved buffers
  and a dirty working tree do not prevent fetching.
- The branch list refreshes and retains its selected ref, including after
  failure or cancellation. Discovering branches not yet known locally belongs
  in an external Git tool; there is no whole-remote fetch action.

`:git-fetch-branch` performs the same action on the selected branch-list row.

#### Pull (`Tab p`)

| Situation | What happens |
| --- | --- |
| A fast-forward is possible | It happens silently: the branch had no commits of its own to lose |
| Both sides have new commits | It shows the drift and offers to replay your commits on theirs |
| Unsaved buffers in the repository | Refused first, since pulled files are reloaded |
| Dirty worktree | Refused |

The offer reads like:
`main and origin/main have both moved on. Press Enter to replay 2 local commits on top of the 1 on origin/main`.
Escape leaves the branch as it was.

- **The replay is a rebase**, so there is never a merge commit whose message
  nothing here could write. It rewrites the replayed commits; they stay
  reachable from the reflog under their old identities, which is why it asks.
- **On conflict it undoes itself:** the rebase is aborted, the working tree
  keeps what it held, and the refusal says so. Runyte has no surface for
  resolving conflicts, so it never leaves you holding one; finish those in
  your usual Git tool.
- **Afterwards** open files are reloaded and gutter bases refreshed, as after a
  checkout.
- **No autostash.** Uncommitted changes are never stashed, even with
  `merge.autoStash` or `rebase.autoStash`. Git reapplies such a stash
  afterwards, and when that conflicts it still exits successfully — leaving
  conflict markers and a stash to recover, with nothing to roll back. A dirty
  worktree is refused up front instead.
- **Fetch and merge run separately**, not as one `git pull`, so an unreachable
  remote is reported as such. The drift offered for replay is read from
  remote-tracking refs, which are only worth reading once a fetch has
  refreshed them.

#### Push (`Tab P`)

- Publishes to the ref the branch tracks.
- The first time, it sets an upstream: `origin` when it exists, or the only
  remote when there is exactly one.
- Nothing forces. A push rejected because the remote has commits you lack is
  reported as such, naming `Tab p` as the way to catch up.

### Background operation and failures

Git discovery, reads, mutations, hooks, fetch, pull, and push run on a bounded
background service, so editing and rendering continue while they queue or run.

**Progress.** A long mutation temporarily replaces the status row with the
action, its target, elapsed time, a cancellation hint, and a rotating
`- \ | /` bar at the right. Every background service using this surface gets
the same spinner.

**Cancelling.** `:git-cancel` stops the current operation. A cancelled
mutation is reported as uncertain and reconciled at once, because cancelling
is not rollback.

**Network safety.**

- Network operations have a two-minute deadline.
- Nothing can prompt: Git's prompts are off, `ssh` runs in batch mode unless
  you set `GIT_SSH_COMMAND`, and no askpass helper falls back to the terminal.
  An authentication needing a password fails with a message instead of
  hanging behind an invisible prompt.

**Environment safety.**

- Repository- and object-selection variables such as `GIT_DIR` and
  `GIT_INDEX_FILE`, and inherited one-shot Git configuration, are removed from
  every child. Starting Runyte from another Git command cannot retarget it.
- File arguments are always literal: a name resembling Git pathspec syntax
  still names only that file.

**Discovery failures.**

- The status line shows the failure with a `:git-refresh` hint, and
  `retrying discovery` while a retry runs.
- `:git-refresh` makes one new discovery attempt in the background. Further
  invocations are unavailable, with a reason, until it finishes. Nothing retries automatically: launch
  failures, signal exits, permissions, and configuration errors all wait for
  you.
- Each of discovery's three local Git reads has a 30-second deadline and
  bounded output. A signal exit does not tell whether the child reached Git.
- Other Git commands stay unavailable. Their palette reasons and the `git` row
  in `:service-health` keep the last diagnostic during a retry.
- A new failure replaces the diagnostic. Success clears it and either
  refreshes the repository or reports that the project is not in a Git
  repository, which offers no retry.
- Failure notifications stay in the notification history.
- Persistent sessions keep the discovery state and notifications across
  detach and reattach; reattaching does not start another attempt.


## Language support

### Syntax highlighting

Tree-sitter grammars are compiled into the binary: no network access, no
grammar directory, and no runtime library loading. Adding a language means
adding a dependency and a row to `src/syntax/grammars.rs`.

#### Language detection

Runyte checks, in order:

1. an exact filename,
2. a registered filename prefix,
3. a case-insensitive extension,
4. a bounded first-line shebang.

| Language name | Detected files include |
| --- | --- |
| `bash` | `.bashrc`, `.bash_profile`; `sh`, `bash`, `ebuild`, `eclass` extensions; `sh`, `bash`, `dash` shebangs |
| `cmake` | `CMakeLists.txt` |
| `make` | `Makefile`, `makefile`, `GNUmakefile` |
| `lua` | `lua` shebangs |
| `dockerfile` | `Dockerfile`, `Containerfile`, their lowercase names, dot-suffixed variants such as `Dockerfile.dev`, and `.dockerfile` / `.containerfile` |
| `kotlin` | `.kt`, `.kts` |
| `xml` | `.xml`, `.svg`, `.xsd`, `.xsl`, `.xslt`, `.wsdl`, `.xaml`, `.csproj`, `.fsproj`, `.vbproj`, `.props`, `.targets`, `.resx`, and `.plist` |
| `hcl` | `.hcl`, `.tf`, and `.tfvars`, including `terragrunt.hcl` and `production.auto.tfvars` |
| `ruby` | `.rb`, `.rake`, `.gemspec`, `.ru`, `Gemfile`, `Rakefile`, `Guardfile`, `Vagrantfile`, `Brewfile`, `Podfile`, `Fastfile`, `Appfile`, `.irbrc`, and `.pryrc`; `ruby` and `jruby` shebangs |
| `php` | `.php`, `.phtml`, `.php3`, `.php4`, `.php5`, `.php7`, `.php8`, and `.phps`; `php` shebangs |
| `elixir` | `.ex` and `.exs`, including `mix.exs`, `config.exs`, and `.formatter.exs` |

Terraform's `.tf.json` files keep JSON highlighting.

#### Notes by language

**INI** parses both `;` and `#` comments. `toggle-comments` inserts `;`.

**Dockerfile** — the language name is `dockerfile`, including in Markdown
fences.

- Instructions, comments, strings, build options, and ports are highlighted.
- Shell commands and `RUN` heredoc bodies use Bash highlighting, within the
  injection size limit. Shell highlighting assumes Bash, even when a file
  selects another interpreter with `SHELL`.

**Kotlin** supports Kotlin 2 multi-dollar strings and guarded `when`
branches. The pinned grammar does not yet model context receivers: it parses
`context(Logger)` as a separate call before the function, so a function text
object starts at `fun` and deliberately leaves out the context-receiver
prefix.

**XML** uses its own parser, not the HTML one, including XML declarations,
entities, and CDATA.

**HCL** highlights blocks, expressions, interpolation, template directives,
and heredocs. HCL fences can use `hcl` or `tf`.

**Ruby** tells local bindings from method calls, including loop, exception,
and pattern bindings. Class/module bodies and methods isolate their locals.
Limitation: in `object = Object.new; def object.greet; end`, the receiver
`object` can take function colouring, because the local-scope query covers the
whole method.

**PHP** highlights code inside `<?php` or `<?=` tags and injects HTML into the
surrounding template, including JavaScript and CSS in script/style elements.

- Heredoc and nowdoc labels naming a bundled language, such as `SQL` or
  `JSON`, highlight their body in that language; unknown labels keep string
  highlighting. Inside an injected heredoc, the embedded language decides the
  colours, so interpolations such as `$id` may look like strings.
- PHPDoc comments keep ordinary comment highlighting.
- PHP snippets in Markdown fences need an opening PHP tag.

**Elixir** highlights offline, without an Elixir runtime or language server.
An `elixir` Markdown fence uses the same grammar within the injection limit.
EEx and HEEx templates are not registered as Elixir, and no Elixir language
server is bundled or configured.

**Missing structural queries.** Dockerfile, XML, HCL, Ruby, PHP, and Elixir
have no dedicated text-object, outline, indentation, or fold queries. In
Elixir, ordinary syntax-tree selection works, but function, class, and
parameter text objects, outlines, syntax indentation, and folds do not.

#### Large files

- Files above 128 KB are highlighted without language injection, so embedded
  languages such as fenced code in Markdown are highlighted only in smaller
  files.
- Markdown's inline grammar is also an injection. Above the limit, headings,
  lists, quotes, and other block structure keep their colours, while emphasis,
  strong text, links, and inline code use the ordinary foreground. The
  injection query still runs over the whole document on every edit, which is
  why this limit remains.
- There is no line or byte limit on highlighting itself. A slow parse delays
  the new tree, not the keystroke that asked for it.

#### Background parsing

Parsing and reparsing run on a background worker.

- A document is readable and editable as soon as its contents load, in
  ordinary text colours until its first syntax tree arrives. Highlighting then
  appears without replacing text or moving the cursor or viewport.
- Opening files, undo/redo, reload, and language changes all use this path.
- If typing outruns the parser, one pending request per buffer is replaced
  rather than queued.
- After the first tree, the previous tree stays in use during updates and its
  visible highlight spans are shifted through pending edits, so colours stay
  visible.
- Completed trees are applied between frames, and only when their parse
  generation, language, and text revision match the live document.
- A parse failure leaves the document editable as plain text; see
  `:service-health`.

**Structural features wait for a current tree.** Outline, folds, matching
brackets, text objects, and structural selection cannot use stale offsets.

- `Space x` commands and `mm` are dimmed while syntax is pending. Invoking one
  reports `Syntax is still parsing`; it is not queued.
- Smart newline falls back to ordinary indentation and list handling.
- Language detection, comments, editing, search, saving, and language servers
  do not need a syntax tree.

### Rendered Markdown

`?` in a Markdown document opens it as a page to read; `?` again returns to
the source. `:render` (also `:markdown`) does the same.

| Source | Rendered page |
| --- | --- |
| `**strong**` | **strong** (bold) |
| `*emphasis*` | *emphasis* (italic) |
| `~~struck~~` | crossed out |
| `# Heading` | Heading without hashes; the first level underlined with a rule |
| `- item` | `•`, then `◦` and `▪` by depth |
| `1. item` | Keeps its number |
| `- [ ]` / `- [x]` | `☐` / `☑` |
| `> quote` | Marked in the margin with `▌` |
| `---` | A rule |
| A table | Re-aligned around `│` |
| A fenced code block | Indented, without fences |
| `[text](url)` | The text followed by its destination, underlined |
| YAML front matter | Kept without its `---` fences, in the dimmed colour |

Anything Runyte does not recognize is passed through as written.

**Moving between the two:**

- Both directions move the cursor to the corresponding text and bring it into
  view.
- On markup removed by rendering, the page uses the nearest surviving text.
  Toggling straight back restores the exact source position; moving around the
  page first takes you to that new place in the source.
- `g f` on a link or image label opens its destination, as in the source.
  Relative paths resolve beside the source document and at the project root.
- `Space e` opens the source document's directory and selects its file.

**The page is a buffer.** It is a generated read-only buffer beside the
document, not a mode the document is in, so both stay open and the source
keeps every editing key.

- Nothing is concealed. The markers are absent because they were never
  written into the page, so no offset moves and rows and columns mean what they
  say.
- Search, selections, splits, and yank work on the page, and find the text
  actually on screen.
- It is rendered from the buffer, not the file, so it shows unsaved work.
  Rendering the same document again reuses and regenerates its one page.

**The scratch buffer** renders too while `editor.scratch_markdown` is on — the
same option that makes `Space p r` refill it as Markdown. Its page is named
`[rendered scratch]`. With the option off, the scratchpad is plain text and
`?` is unbound there.

**Tables and soft wrap.** `Space p s` also wraps rendered tables.

- Columns share the pane's text width, keeping short columns compact and
  wrapping longer cells independently.
- A row is as tall as its tallest cell. Vertical separators continue through
  its lines, and horizontal rules separate entries.
- Resizing changes only the visual layout. With soft wrap off, tables keep
  their natural widths.
- Columns keep a small minimum width. If the pane is too narrow for them all,
  the table scrolls horizontally; moving the caret into a hidden column
  brings it into view. Other text still wraps normally.
- Up/down follows visual rows and skips added rules. In a shorter cell's blank
  continuation, it moves to the nearest cell with text on that row.
- Clicks on padding place the caret at nearby text; added rules are not click
  targets.
- Search and yank use the logical text: wrap breaks, padding, and added rules
  are not copied. A selection across cells follows that order, so it need not
  be a rectangle on screen.

**Attributes and colours.** Bold and underline are drawn by every terminal,
italic by most. Each role also has a theme colour, so a terminal that ignores
an attribute still shows the difference. The roles are the `markup.*` entries
in [Syntax colours](#syntax-colours), so a theme can recolour them.

### Language servers

#### Setting up a server

`rust-analyzer` is configured out of the box. For another language:

1. Install the server so it is on `PATH`.
2. Add it under `lsp` in the configuration file.
3. Run `:config-reload`.
4. Make sure the workspace allows language servers: answer the prompt shown
   when it first opened, or run `:lsp-trust` (see
   [Permission to run servers](#permission-to-run-servers)). A reload does not
   ask again.

```yaml
lsp:
  enable: true
  markdown:
    command: marksman
    args: ["server"]
```

| Field | Meaning |
| --- | --- |
| `command` | Executable name or absolute path |
| `args` | Optional list of arguments. The server is started directly, not through a shell. |
| `initialization_options` | Optional; passed verbatim as `initializationOptions` |

**Language keys** are Runyte's language names, so a buffer's language is the
same question for highlighting and for LSP: `rust`, `python`, `swift`, `c`,
`cpp`, `javascript`, `typescript`, `tsx`, `html`, `css`, `go`, `bash`, `java`,
`kotlin`, `json`, `sql`, `lua`, `c-sharp`, `zig`, `cmake`, `proto`, `make`,
`ini`, `toml`, `yaml`, and `markdown`. Other keys under `lsp` are rejected,
apart from `enable` and the `servers` wrapper.

- `lsp.servers.<language>` is an accepted compatibility spelling of
  `lsp.<language>`, which is preferred.
- Copy-ready examples for the servers in Runyte's compatibility tests are in
  [docs/lsp/](lsp/README.md).
- Server definitions are YAML-only. `Space o o` can only turn LSP on or off as
  a whole.
- `:help lsp` repeats this setup inside the editor.

**Reloading and restarting:**

| Command | Effect |
| --- | --- |
| `:config-reload` | Adopt new definitions; restart only languages whose definition changed |
| `:lsp-restart [language]` | Restart a stopped server from the configuration already loaded. It does not re-read YAML, so it is the wrong command after editing the file. |
| `:lsp-status` | Show servers that started or failed |
| `:service-health` | Show whether the active document has a configured and attached server |

`lsp.enable` is decided at startup. To turn LSP on or off, reopen standalone
Runyte, or restart a persistent session with
`runyte --session-restart [WORKSPACE]` (passing the same `--config PATH` if
the host used a non-default one).

**Failures cost only the language features.** A server that is missing,
crashes, or answers slowly never makes the editor wait. Diagnostics from a
stopped server are dropped rather than left as stale claims. A launch error
appears in `:lsp-status` after the first start attempt and in the
notification center.

#### Permission to run servers

The first time a workspace opens with LSP enabled, Runyte asks whether
language servers may run there.

| Choice | Lasts |
| --- | --- |
| **Keep LSP disabled** (default) | — |
| **Allow LSP once** | Until the standalone editor or persistent host stops |
| **Always allow LSP** | Remembered for this exact workspace |
| Escape | Dismisses without changing permission; on a first visit LSP stays off |

Editing, search, and highlighting work either way.

**Why it asks.** Language servers can run project code — build scripts,
plugins — with your permissions. Approval covers every configured server,
custom ones included, and future code changes in the workspace. It is
permission to run those tools, not a security review or a sandbox.

**Changing your mind.** `:lsp-trust` reopens the choices, including
revocation.

- Disabling LSP there stops this host's servers and clears their editor state.
  It cannot undo code already run.
- A persistent host owns the permission, so detaching and reattaching cannot
  bypass it. Other running editors keep their own decision until changed or
  restarted.
- Setting `lsp.enable: false` suppresses the question and keeps approval from
  starting LSP.

**Where decisions are stored.** Private `lsp-trust/` storage under Runyte's
per-user platform cache, separate from `.runyte/`.

- Clearing that cache makes Runyte ask again.
- A workspace rooted at your home directory may use Runyte's standard cache
  under it; other project-local cache overrides are rejected.
- Records use the canonical workspace root: symlinked aliases share a
  decision, but nested workspaces, other worktrees, and clones elsewhere do
  not.

**When storage fails:**

- An unreadable decision keeps LSP disabled and asks again.
- If storage is unavailable, the prompt explains why and offers only **Keep
  LSP disabled for now** and **Allow LSP once**.
- A failed save keeps the prompt open with the error and those temporary
  choices.
- **Keep LSP disabled for now** stops this editor's servers without changing
  any remembered decision.
- **Allow LSP once** needs no new storage, but first removes an older
  remembered decision; if that fails, the choice stays pending. The next
  launch asks again.
- Reopening `:lsp-trust` retries the store once it is repaired.

#### What a server is asked

Runyte only sends requests for capabilities the server advertised in its
handshake. Hover, completion, signature help, goto (definition, declaration,
type definition, implementation), references, document and workspace symbols,
rename, code actions, and formatting are gated independently.

- Asking for one the server never advertised reports it as unavailable on the
  interaction line. It is not kept as a notification, since it is expected
  rather than a fault. Typing near a trigger character a server does not
  support costs no round trip.
- A `Method not found` from a server that *did* advertise the capability is a
  protocol violation, reported and kept as an ERROR.

**Trigger characters** come from the server too:

| Server | Completion after | Signature help |
| --- | --- | --- |
| clangd | `/`, `"` | Seven delimiters |
| Pyright | `[`, `"` | Among its characters, names `)` but answers nothing there, closing the popup |
| rust-analyzer | `'` | |
| gopls | | Two delimiters |
| sourcekit-lsp, Marksman | | None |
| A server advertising the capability but naming no characters | `.`, `:` | `(`, `,` |

- The table names only the characters discussed here, not each server's full
  list; a blank cell says nothing.
- Some servers name the closing `)`, so the inner `)` of `f(g(a), b)` asks
  again instead of dismissing the popup; clangd and
  typescript-language-server answer with the enclosing signature.
- A server may name retrigger characters, active only while a popup shows.
  Runyte advertises the `contextSupport` that allows them and tells the server
  which character asked and whether a popup was open.
- A server naming `)` neither way has the popup closed locally.
- `tests/lsp_real_servers.rs` records what each server in the compatibility
  matrix does.

#### Edits from a server

- Servers may not create, rename, or delete files. Such operations in a
  workspace edit are reported and skipped, and files a rename touches are
  opened as buffers rather than written behind your back.
- Text edits must name absolute local files inside the project and exact,
  forward-ordered character boundaries. A malformed, remote, out-of-range, or
  overlapping edit rejects the whole multi-file change.
- Open target buffers are checked against the revisions the request was made
  from, including targets other than the source buffer.
- Versioned diagnostics are ignored once their document has moved on.

#### Compatibility tests

An opt-in Docker matrix tests real servers: Python (Pyright), Swift
(SourceKit-LSP), C and C++ (clangd), JavaScript (typescript-language-server),
Go (gopls), Rust (rust-analyzer), and Markdown (Marksman).

```sh
tests/lsp/run.sh
```

- The image pins every server toolchain. Each test uses Runyte's production
  stdio transport for the full handshake, in a disposable project.
- After initial symbol and definition checks, each sends an unsaved
  incremental change with non-ASCII text and resolves a definition that exists
  only in the changed document.
- Fixtures then exercise completion, hover, signature help, references,
  rename, formatting, code actions, and diagnostics, as far as each pinned
  server meaningfully implements them. `tests/lsp_real_servers.rs` marks each
  feature as tested, advertised-only, or unsupported per server.
- Rust, Go, and JavaScript also resolve definitions across files.
- Diagnostic fixtures check the reported range and the publication that clears
  it after a fix.
- Ordinary `cargo test` skips the matrix: building its image downloads several
  large toolchains, and running it starts eight real servers.

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
| `Tab` | Code actions for the selection |

**Hover documentation** stays anchored to its source.

- A short document is a peek: it closes and passes the next key on.
- With more than twelve lines, the title says how much was left out, and Enter
  opens the full text in a retained read-only `[documentation]` buffer.

**Goto and pickers.** One result moves the selection to it; several open a
picker. Every picker filters as you type, moves with the arrows or
`Ctrl-n`/`Ctrl-p`, opens with Enter, and closes with Escape.

**Undo.** A completion that needs an import applies both edits as one undo
step, and so does a rename across a file. Everything typed between entering
Insert mode and returning to Normal is one undo step too; undo and redo map
the caret through the inverse edit rather than leaving it at a stale offset.

#### Completion

Three sources share one popup:

| Source | Appears | Needs |
| --- | --- | --- |
| Language server | After one of the server's trigger characters, on `Ctrl-x`, or with `Space l c` | A server |
| Path | When the text before the caret is a valid directory followed by part of a name | Nothing |
| Word | When a typed prefix reaches `editor.word_completion_minimum` characters (3) | Nothing |

| Key | Effect |
| --- | --- |
| `Tab` | Accept |
| `Escape` | Dismiss |
| Enter | Always inserts a newline and dismisses the popup |

**Why Enter never accepts.** A popup can open on its own, after a trigger
character or any three-character word prefix, so Enter always keeps its usual meaning
rather than risking an unwanted candidate on a keystroke meant to end a line.

**Escape** dismisses the popup and returns to Normal mode in one press for
automatic word completion, and in an editable explorer, where a row's `/` can
open path completion unasked.

**Language-server completion:**

- An explicit `Ctrl-x` includes the identifier already before the caret, uses
  the server's `filterText` and `sortText` when present, and stays the active
  source until space, newline, acceptance, or dismissal.
- Typing punctuation does not hand the session to word or path completion. A
  trigger character refreshes the candidates for the new context.
- If the prefix has no matches, the popup hides without ending the session,
  so Backspace can bring matches back. Backspace and Delete keep the session;
  moving the caret or another editing command ends it.
- `Ctrl-x` replaces a word popup as soon as the request is sent.

**Path completion:**

- It opens on any keystroke that leaves a valid directory plus a name
  fragment before the caret, not only on `/`, so editing an existing path
  shows hints just like typing a new one.
- Relative paths resolve against both the active file's parent and the
  project root, so `dir/`, `files/`, `./files/`, and `../files/` work from
  whichever makes them valid.
- Directories keep a trailing `/`; accepting one offers its children.
- A large directory shows its first few hundred names, but typing more narrows
  against the whole directory, so every existing name is reachable.
- A path in progress takes over from word completion as soon as `/` is typed,
  unless an explicit LSP session is active.
- Command palette path arguments work the same way.

**Word completion** offers words from every open buffer in the workspace,
including the one being edited.

- Every open buffer contributes, including explorers, whose entries are file
  names.
- It appears in file buffers, the scratch buffer, and the commit message.
- A word is Unicode letters and numbers. A hyphen stays inside only when it
  joins characters on both sides, so `up-to-date` stays whole. Other
  punctuation is a boundary and never included.
- Words from the current buffer come first, ordered by how often they occur
  there, then words from every other buffer in the same order. The word being
  typed is never offered to complete itself.
- A background index keeps the list, so a candidate can be one keystroke
  stale, but typing never waits for it.
- It never overrides `Ctrl-x` or a path in progress.
- Turn it off, or change the trigger length, with `editor.word_completion`
  and `editor.word_completion_minimum` in `Space o o`.


## Workspaces and persistent sessions

A **workspace** is one project directory and its editor scope. It exists in
both modes:

| Mode | Where live editor state lives | After the TUI exits |
| --- | --- | --- |
| **Standalone** (default) | The TUI process | Gone |
| **Persistent** | A local host process, the **persistent session** | Kept: open and unsaved buffers, selections, registers, syntax state, diagnostics, Git projections, language-server processes, and terminal sessions |

Persistent sessions work on Unix and Windows; see
[Windows support](#windows-support) for the Windows differences.

### Starting a workspace

| Command | Effect |
| --- | --- |
| `runyte` | Open the workspace discovered from the current directory, in the mode set by `workspace.mode` |
| `runyte --persistent`, `runyte -a` | Attach to the current project's persistent session, starting it if needed |
| `runyte -a WORKSPACE` | Attach to a named session from any directory, starting it if needed |
| `runyte --standalone` | Standalone, overriding `workspace.mode: persistent` |
| `runyte --init /path/to/project` | Make that exact directory a standalone workspace root and open it |
| `runyte DIRECTORY` | Open that directory in the workspace discovered from the current directory |
| `runyte --serve` | Run the host in the foreground |

**Finding the workspace.** Runyte walks up from the launch directory to a Git
root, or else to a workspace state directory (`.runyte/`). If neither exists,
it asks where project data should live. See
[Where workspace state lives](#where-workspace-state-lives).

**`--init`** creates the configured state directory (`.runyte/` by default)
when absent, then opens that directory.

- An existing state directory is used as is, never reset or removed.
- It picks the named directory even when an ancestor has its own state
  directory.
- Use it only when the directory itself must be the standalone workspace root.
  `runyte DIRECTORY` does not change the workspace.

**`workspace.mode: persistent`** makes a bare `runyte` attach like
`runyte -a`.

- Launches naming a target — a file or a directory, including `runyte .` — stay
  standalone, so their relative paths and `+LINE[:COLUMN]` positions keep
  ordinary meaning.
- `--persistent` reads its argument as a workspace, not a file.

**`runyte -a WORKSPACE`** uses the same selector as the lifecycle commands (see
[Selecting a session](#selecting-a-session)).

- A session that is not running is started first.
- An existing directory unknown to the catalog names that exact directory:
  Runyte creates its state directory if needed and starts its session.
- A bare explicit `runyte -a` does the same for the current directory when no
  workspace is discoverable there.

**Only one interactive TUI** may be attached at a time. Separate control
connections can still manage the host.

### Quitting and detaching

| Command | Standalone | Persistent |
| --- | --- | --- |
| `:quit` from the last pane | Exits | Stops a clean session and returns the TUI to the previously visited running session |
| `:quit-all` | Exits | The same, whatever the pane count |
| `:quit-here` | Exits and changes the shell directory, through the wrapper | The same; without the wrapper both refuse. See [Change the shell directory on exit](#change-the-shell-directory-on-exit) |
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
| `Space 1`–`Space 9`, `:session-1`…`:session-9` | Attach directly to the numbered running session |
| `Shift-Left` / `Shift-Right` | Visit the previous / next running session in manager order |
| `Ctrl-w a`, `:previous-session` | Alternate between the last two attachments |
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

**In standalone mode** (Unix) there is no host, so the session namespace is
inert rather than a set of commands that each refuse. `Space Space` and
`Space 1`–`Space 9` are greyed in the key hints, `:session-list`,
`:session-attach`, `:session-stop`, and `:session-rename` are greyed in the
command palette, and invoking one answers `needs workspace.mode: persistent`.

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
- Exited terminals are in `Space t t`, whose Tab menu offers **Close all
  exited terminals** without touching live children.

The Navigator, the buffer list (`Space b b`), and the terminal list
(`Space t t`) are one list over three scopes — every open destination, the
buffers, or the terminals including exited ones — with the same columns:

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

**Previous destination.** `Ctrl-w p` (`:previous-destination`) returns to the
previous open destination in this pane, skipping closed resources and exited
terminals.

#### Switching sessions

- **`Shift-Left` / `Shift-Right`** visit the previous/next running session in
  manager order, including unnumbered ones, wrapping at the ends. They work in
  Insert and Terminal Insert as remappable exceptions to child input, subject
  to open overlays and confirmations. They never start a stopped session or
  force an occupied attachment. If an outer tmux takes Shift-Left/Right,
  release or change those tmux bindings.
- **`Ctrl-w a`** (`:previous-session`) alternates between the last two
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
switches the outer TUI to that directory.

- Relative arguments resolve from the shell's directory.
- The command returns to the shell without starting a nested TUI; switching
  back returns to that shell.
- The route checks the owning host, terminal, and caller, and reports the
  attachment or its error. Stale, detached, or standalone parent contexts get
  an actionable error.
- Outside integrated terminals, launching works as usual.

### Managing sessions from the command line

```sh
runyte --persistent [WORKSPACE]         # or runyte -a [WORKSPACE]
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
  copied, stale, or standalone parent marker is refused.
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
- Switching sessions keeps the request pending in its host. Explicit detach or
  loss of the caller cancels it; buffers are kept and no nested TUI takes over
  the terminal.

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

Save [runyte.ps1](../contrib/runyte.ps1) somewhere stable and dot-source it
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
- In persistent mode `:quit-here` stops the session after the same checks as
  `:quit`. Use `:detach` to keep the host running.


## Commands

### The command palette

`:` opens the command palette: every command with its aliases, usage, and a
short description, grouped by category.

| Key | Action |
| --- | --- |
| Type | Filter by name, alias, description, or category |
| Up / Down | Select a result |
| Tab | Complete it |
| Enter | Run it |
| Escape | Close the palette |

- Commands whose service is unavailable for the active buffer stay listed but
  dimmed, with a reason. Running one leaves the typed command and editor state
  alone.
- `::` opens the plugin-only palette; see [Plugins](#plugins).

#### Path arguments

Once a command taking a path is selected, the rows become filesystem hints,
from the editor working directory or from the absolute path being typed.

- Directories come first, with a trailing separator. Completing one keeps the
  palette open for its children. Running `:open` on a directory opens the
  explorer.
- `~` means your home directory, so `:open ~/.bashrc` and `:open ~/projects`
  work without a shell.
- Dotfiles are offered when the name typed begins with `.`, or when hidden
  files are enabled.

**Enter in path prompts** behaves the same everywhere: the palette's path
arguments, the `Space / p` finder path, and plugin fields that complete local
paths.

| Typed path | Enter |
| --- | --- |
| Does not name an existing file or directory, with hints showing | Accepts the selected hint, like Tab |
| Names an existing file or directory | Submits |

- `:cd sr` Enter Enter completes to `src/`, then changes into it.
- A new name that is a prefix of an existing one is completed too:
  `:w notes` beside `notes.md` becomes `:w notes.md`. Hints follow only a
  cursor at the end of the line, so press Left, then Enter, to submit `notes`
  as typed.
- Completion inside a buffer — words, language-server items, paths — still
  uses only Tab; Enter there inserts a newline.

#### The working directory

The working directory starts where Runyte was launched.

- `:cd <path>` changes it. Relative paths resolve from the current working
  directory. With an explorer active, `:cd` also retargets that explorer; from
  a file, the file stays open.
- `Space E` opens the working directory. `Space e` opens the active buffer's
  directory and selects its file, so Enter returns to it; from a rendered
  Markdown page it uses the source document. A pathless buffer falls back to
  the working directory.
- `Space f` (or `Space / f`) always opens the project Finder in name mode, and
  `:fuzzy-grep` its content mode.
- `:file-picker-directory` and `:fuzzy-grep-directory` search the active
  file's parent, the active explorer's root, or the working directory for a
  pathless or generated buffer.

### Command list

#### File and buffer commands

| Command | Aliases | Effect |
| --- | --- | --- |
| `:open <path>` | `e`, `edit` | Open a file or directory in the active pane |
| `:write [path]` | `w`, `save` | Save, optionally choosing a path |
| `:write! [path]` | `w!`, `save!` | Save, replacing an existing file or one that changed on disk |
| `:write-buffer-close` | `wbc` | Save and close the buffer in place |
| `:close` | `c`, `buffer-close`, `bc`, `close-buffer`, `cb` | Close the active buffer in place |
| `:close!` | `c!`, `buffer-close!`, `bc!` | Discard unsaved text and close the active buffer |
| `:buffer-new` | `new` | Open a new scratch buffer in the current pane |
| `:reload` | | Reload the active local/provider file, or refresh the explorer or supported Git list |
| `:path` | | Show the active buffer's absolute path in a wrapped popup; Tab offers copying it to the system clipboard (`s`) or the unnamed Runyte register (`r`) |
| `:cd <path>` | | Change the working directory; retarget an active explorer |
| `:explorer [path]` | `files` | Open an editable directory explorer |
| `:render` | `markdown` | Render a Markdown document, or return to its source |
| `:diff-disk` | | Compare a fresh disk snapshot with the active file buffer |
| `:diff-remote` | | Compare a fresh remote snapshot with the active provider document |
| `:diff-this` | `difft`, `dt` | Mark this buffer, or compare it with the one marked before it |
| `:diff-off` | `do` | Close the comparison this buffer is part of |
| `:pipe <shell-command>` | `\|` | Replace each selection with the command's stdout |
| `:pipe-cancel` | | Cancel the workspace's running pipe job |

#### Finder commands

| Command | Effect |
| --- | --- |
| `:file-picker` | Open the Finder over files, buffers, and terminals |
| `:file-picker-directory` | Fuzzy-find below the active file/explorer directory |
| `:open-explorer-finder` | Open the Finder at the active explorer's directory |
| `:open-explorer-system` | Open the explorer's directory in the system file manager |
| `:fuzzy-grep` | Open the Finder in content mode |
| `:fuzzy-grep-directory` | Fuzzy-search contents below the active file/explorer directory |
| `:outline` (`document-outline`) | Open the Tree-sitter document outline |

#### Pane and view commands

| Command | Aliases | Effect |
| --- | --- | --- |
| `:vsplit [path]` | | Create a side-by-side split |
| `:hsplit [path]` | `split` | Create a stacked split |
| `:window-close` | `wc` | Close the active pane, but not the last one |
| `:resize-right +/- N` | | Grow or shrink the pane at its right edge by N cells |
| `:resize-left +/- N` | | Grow or shrink the pane at its left edge by N cells |
| `:resize-top +/- N` | | Grow or shrink the pane at its top edge by N cells |
| `:resize-bottom +/- N` | | Grow or shrink the pane at its bottom edge by N cells |
| `:zen` | | Toggle a centred, maximized editable writing viewport |
| `:fullscreen` | | Toggle the active pane across the whole editor area, at its ordinary width |

#### Quit and session commands

| Command | Aliases | Effect |
| --- | --- | --- |
| `:quit` | `q` | Close the pane and its unique buffer, or stop safely from the last one |
| `:quit!` | `q!` | Discard its unique buffer, or force quit from the last pane |
| `:quit-all` | `qa` | Quit safely regardless of pane count, without ending terminals |
| `:quit-all!` | `qa!` | Discard buffer changes and quit, without ending terminals |
| `:quit-here` | `qh` | Quit and return the shell to the active directory |
| `:quit-here!` | `qh!` | Discard changes, quit, and return there |
| `:write-quit` | `wq` | Save, then close the pane or quit from the last one |
| `:detach` | | Disconnect this persistent TUI while keeping all editor state |
| `:session-list` | `sl` | Open the session manager (Unix persistent mode or Windows native controls) |
| `:session-1` … `:session-9` | | Attach directly to the numbered running persistent session |
| `:session-attach WORKSPACE` | `attach` | Attach to another workspace's persistent session |
| `:session-stop [WORKSPACE]` | | Stop a clean persistent session |
| `:session-rename WORKSPACE NAME` | | Rename a persistent session |
| `:session-clean` | | Clean verified stopped session history (Windows) |
| `:previous-session` | | Return to the previously visited persistent session |
| `:previous-destination` | | Return to the previous destination in this pane |

#### Terminal commands

| Command | Aliases | Effect |
| --- | --- | --- |
| `:terminal [command]` | `t`, `term` | Run a program in this pane, or `$SHELL` |
| `:terminal-file-directory [command]` | | Run from the active file's parent |
| `:terminal-directory-root [command]` | | Run from the active explorer root |
| `:terminal-selected-directory [command]` | | Run from the selected directory entry |
| `:terminal-session-directory <number\|name>` | | Run a shell from another terminal's safe directory |
| `:terminals` | | List the running terminals and show one here |
| `:terminal-show <number\|name>` | | Show a terminal in this pane |
| `:terminal-rename <name>` | | Name this pane's terminal |
| `:terminal-output` | | Copy this terminal's output into a read-only buffer |
| `:terminal-send [number\|name]` | | Send the selection, or the whole buffer, to a terminal |

#### Git commands

| Command | Effect |
| --- | --- |
| `:git-status` | Open the changed-file list |
| `:git-diff` | Show the active file's unstaged diff |
| `:git-diff-side-by-side` | Compare the active file's complete Git versions |
| `:git-index` | Review everything staged for the next commit |
| `:git-stage` | Stage the active file, or every file selected in the list |
| `:git-unstage` | Unstage the active file, or every file selected in the list |
| `:git-stage-hunk` | Stage the exact hunk under the cursor |
| `:git-unstage-hunk` | Unstage the exact staged hunk under the cursor |
| `:git-stage-lines` | Stage the supported saved source-line selection |
| `:git-discard` | Throw away a file's uncommitted changes, after a confirmation |
| `:git-commit` | Write a message and commit what is staged |
| `:git-branches` | Open the local and cached remote branch list |
| `:git-fetch-branch` | Fetch the selected cached remote branch or local upstream |
| `:git-worktrees` | Open the repository worktree list |
| `:git-compare` | Compare committed tips with the selected branch or worktree |
| `:git-log` | Open the Git log, or refresh it from its first page |
| `:git-search-commits` | Fuzzy-search commits by message, ID, author, or date with a full-message preview |
| `:git-blame` | Show live-buffer attribution for the primary line |
| `:git-blame-file` | Open full-file live-buffer attribution |
| `:git-stashes` | Open or refresh the bounded stash list |
| `:git-stash-tracked <name>` | Stash a tracked-worktree snapshot, leaving the index applied |
| `:git-stash-all <name>` | Stash tracked worktree and index changes, after a confirmation |
| `:git-stash-untracked <name>` | Stash tracked changes and untracked files together |
| `:git-stash-apply` | Apply the selected stash without dropping it, after a confirmation |
| `:git-stash-drop` | Drop the selected stash, after a confirmation |
| `:git-refresh` | Refresh Git state or retry failed repository discovery |
| `:git-cancel` | Stop the active Git operation and reconcile mutations |

#### Language server commands

| Command | Aliases | Effect |
| --- | --- | --- |
| `:format` | `fmt` | Format the active buffer |
| `:lsp-trust` | | Choose workspace permission to run language servers |
| `:lsp-restart [language]` | | Restart stopped language servers |
| `:lsp-status` | | Report language server state |

#### Configuration, help, and diagnostic commands

| Command | Aliases | Effect |
| --- | --- | --- |
| `:config` | `settings` | Open the settings menu |
| `:config-reload` | | Re-read the loaded configuration file into this session |
| `:theme [name]` | | Choose a theme in the settings menu, or switch straight to the named one |
| `:help [topic]` | `?` | Open the general manual, optionally at a named section |
| `:about` | | Show Runyte's logo, version, and getting-started guide |
| `:tutorial [reset\|sessions]` | | Open or resume the tutorial; `reset` starts over and `sessions` opens its persistent-session lesson |
| `:grammar [runyte]` | | Report the active Runyte editing grammar |
| `:notifications` | `not` | Open retained notification history |
| `:service-health` | `health` | Inspect syntax, LSP, providers, and helper health |
| `:log-open` | | Open the diagnostic log owned by the process that holds this workspace |
| `:context-access [identity]` | | Review, grant, inspect, or revoke agent context access |

#### Plugin commands

| Command | Effect |
| --- | --- |
| `:plugins` | Open the configured plugin manager |
| `:plugin-stop <id>` | Stop a configured plugin or cancel its pending restart |
| `:plugin-restart <id>` | Restart a configured plugin after its cleanup finishes |


## Plugins

Plugins are external programs that Runyte starts and talks to. The
[plugin guide](plugins.md) is the full reference: a runnable example,
installation and configuration, command bindings, errors, subscriptions,
limits, and the stable API.

- There is no package manager and no automatic discovery. A plugin runs only
  when explicitly enabled in the configuration.
- Enabled programs run with your permissions. They are not sandboxed.

### Using plugins

| Key or command | Action |
| --- | --- |
| `::` | Open the plugin-only command palette, with completion and descriptions |
| `:plugins` | Open the plugin manager |
| `:plugin-stop <id>` | Stop a plugin, or cancel its pending restart |
| `:plugin-restart <id>` | Reconnect after cleanup finishes |
| `Tab` in an application view | Actions from its plugin |

**Plugin commands:**

- Plugins can declare short names, such as ru-time's `::time`, `::time-add`,
  and `::time-delete`. Full `:plugin.<id>.<command>` names always work.
- Conflicting short names are disabled for all claimants and reported in
  `:notifications`. Stopping a claimant restores a name once it is unique.
- Backspace over the second colon returns to the ordinary palette.
- Keys for plugin commands are set with `plugins[].bindings`.

**The manager** (`:plugins`) lists configured, disabled, and failed plugins.
Select one to see its capabilities, jobs, activity, helpers, and diagnostics;
Enter offers lifecycle actions.

- Restart does not replay old actions.
- Stopping keeps retained views readable and dirty provider documents
  editable.
- Persistent hosts keep plugin processes across detach and reattach.

### What plugins can do

**Edit text.** Explicitly enabled process plugins can register commands, read
the invoking buffer's text and selections, and return replacements as one
undoable edit. They run asynchronously; if the text changed meanwhile, the
stale result is rejected.

**Application views.** Plugins can provide retained native views, typed
commands, finite background jobs, explicit buffer and selection operations,
and bounded local filesystem operations with native confirmation.

- **Tab menus** can depend on the selected rows: a connected database can offer
  Disconnect while a disconnected one offers Connect. Multiple rows offer only
  actions common to all. Moving the selection after opening the menu requires
  reopening it, and changed models are checked again before an action runs.
  Other plugins keep view-wide menus.
- Actions can have readable labels and be grouped into named sections.
  Headings cannot be selected; typing filters across sections. Enter can open
  the selected item even when its callback is hidden from the menu.
- View titles identify the content, and plugins can place labelled facts,
  such as filters or a database path, above it.
- **Read-only documents** open on demand and scroll, search, and copy as one
  buffer, even when received in chunks. The supported limit is 8 MiB and
  250,000 lines; loading can fail while the host's shared memory allowance is
  in use. A shortened preview stays a preview until the full content is
  published.

**Prompts and forms.** Native prompts, filterable choices, and forms with
masked secret fields.

- A text or secret field rejects a whole paste containing control characters
  (including a trailing newline) or exceeding its byte or character limit.
  The value and cursor stay unchanged, and feedback names the cause.
- Required fields and minimum lengths are checked on submit.
- Fields can opt into asynchronous validation after Enter. The form stays open
  while you edit, stale results are rejected, and input never blocks. Secret
  values reach validation only when that field opts in.
- Text fields can opt into local path completion: type a directory or name
  prefix, choose with Up/Down, and press Tab. Directory suggestions continue
  into the directory. Shift-Tab returns to the previous field and Escape
  cancels. Enter completes the selected suggestion while the path does not
  name an existing entry, and submits once it does. Relative paths start at
  the workspace root; spaces and quotes are literal; `~` and environment
  variables are not expanded. Older hosts keep ordinary text fields.

**Documents.** Applications can create named unsaved documents and save them
asynchronously. Later edits stay dirty, and pending or uncertain writes
cannot silently complete a close or `--wait` request.

**Remote documents** (providers) open version-bound UTF-8 documents with
normal editing, highlighting, and remote saves.

- Providers with conditional writes save normally. Weaker providers need
  native confirmation describing the overwrite race and replacement
  guarantee. Cancelling keeps the text and undo.
- Newer edits stay dirty during uploads; save-and-close waits for confirmed
  success.
- An explicit rebind reconciles restarted providers and uncertain writes.
- `:diff-remote` compares fresh remote text with your edits without changing
  the saved baseline; both sides must fit the 4 MiB comparison limit.
- `:reload` reads fresh remote text. Dirty or uncertain documents offer reload
  (keeping your edits against a new baseline) or cancel. Reload is one
  undoable edit and keeps earlier history. Unknown writes need provider
  settlement before a baseline is accepted. Each side may hold at most 8 MiB.
  See [provider recovery](plugins/applications.md#native-reload-and-conflict-recovery).

**Subscriptions.** Applications can subscribe to buffer, pane, owned
view/job, and attachment metadata, with consistent baselines and bounded,
ordered delivery. New buffers are reported without polling; slow consumers
get an explicit resynchronization marker.

**Helpers.** Applications can launch
[managed helpers](plugins/applications.md#managed-helpers) with bounded binary
input and retained output, kept separate from the plugin protocol.

- Closing a helper, or stopping its plugin, cleans up and reaps the owned
  process tree. Output from a naturally exited helper stays readable until
  released.
- Idle enabled helpers alone do not prevent persistent host retirement.
- The runnable helper controller shows Send, Flood, EOF, and Close actions
  without a network service.

**Notifications and handoffs.** Applications can publish owner-labelled
notifications without changing focus or action feedback. Explicit native
handoffs can open a terminal session, an HTTP/HTTPS URL in the browser, or a
workspace file with its system handler. A terminal handed over survives
plugin stop and follows normal terminal retention. See the
[handoff example](plugins/applications.md#notifications-and-native-handoffs).

**Activity leases.** Continuing playback or service work can hold a renewable
[activity lease](plugins/applications.md#continuing-activity) of at most ten
minutes per grant.

- Session health and `:service-health` show its owner, title, and state.
- Active leases, and their two-second cancellation grace, protect against
  normal quit and idle retirement. Detach keeps them running.
- An owner that does not acknowledge expiry or cancellation is stopped, with
  its managed helpers cleaned up.
- `:q!` does not bypass this. Stop the owner in `:plugins` before quitting, or
  `:detach` in persistent mode to leave it running. A forced persistent-session
  stop remains available.

**Settings and state.** Applications can read their configured settings and
save bounded, non-secret workspace preferences through the
[settings and state API](plugins/applications.md#settings-and-workspace-state).
Updates check the revision the application saw and keep existing data on
conflict. The runnable `preferences.py` example shows a configured default
and a saved destination that survives restarting the editor. Secrets belong in
a credential manager or authentication helper.

### Example applications

The [application guide](plugins/applications.md) has runnable examples and
lists remaining work:

| Example | What it shows |
| --- | --- |
| [SFTP browser and editor](plugins/applications.md#sftp-browser-and-editor) | Remote files over SSH |
| [FTP/FTPS browser and editor](plugins/applications.md#ftp-and-ftps-browser-and-editor) | The same workflow over standard-library FTP/FTPS |
| [Local media controller](plugins/applications.md#local-media-controller) | Audio and video playback through mpv |
| [Todo showcase](plugins/todo/README.md) | Matching Python, Rust, and C task-list applications |
| Task list, local file manager, document, memory provider, background jobs | Other runnable examples in the guide |

**SFTP.** Verifies SSH host keys, uses explicit identity files or an existing
SSH agent, and opens remote UTF-8 documents up to 8 MiB. Run `::sftp .` with
its documented profile; Enter opens a file, and `:write` confirms the remaining
remote overwrite race.

**FTP/FTPS.** Defaults to certificate-verified FTPS with encrypted data
connections. Plain FTP is an explicit profile choice and is labelled in the
UI. Saves need native confirmation of non-atomic replacement, and credentials
come from an explicit private file.

**Both remote browsers:**

| Action | Flow |
| --- | --- |
| `download` | Asks for a new local path and stages up to 8 MiB of file bytes. When the browser shows `Download ready`, `confirm-download` opens the native review. Downloads never publish automatically; `cancel-download` cancels unaccepted work. |
| `upload` | Freezes the bytes of a workspace disk file of at most 8 MiB; `confirm-upload` opens native confirmation for the remote destination. Unsaved editor text is not the disk file. `cancel-upload` cancels pending work; an unknown promotion outcome stays visible and blocks another upload until an explicit plugin restart. See [binary uploads](plugins/applications.md#binary-uploads-from-workspace-disk-files). |
| `mkdir`, `rename`, `delete` | Act on one remote file or empty directory. `confirm-operation` opens native confirmation; `cancel-operation` cancels. Deletion is permanent, checks are best effort, and an unknown outcome must be inspected before retrying. See the [remote operation guide](plugins/applications.md#confirmed-remote-directory-operations). |

Remote identities stay separate from local files, and open remote documents
keep their identity and unsaved text.

**Media controller.** Plays workspace audio and video through an installed
mpv, with a native playlist: play, pause, relative seek, next, previous, and
stop. Video appears in mpv's own window. Playback continues across detach and
protects the persistent session while active; pausing releases that
protection, and stop closes the player. The
[service adapter guide](plugins/media-services.md) covers Spotify
prerequisites and YouTube browser playback separately.

## Agent access to workspace context

The optional [Runyte context bridge](../bridges/runyte-context/README.md) lets
AI agents read integrated terminals, unsaved buffers, selections, and pane
viewports in workspaces you authorize. It runs as a separate local MCP
process, outside the editor.

### Setting it up

1. Install the bridge and configure each agent, following the bridge's
   instructions.
2. In each workspace the agent should see, run `:context-access`.
3. Choose the scopes to grant and apply them.

`:context-access codex` (an optional identity name) gives separate bridge
installations separate grants and revocation.

**Platforms.** Context services start with normal workspace startup on Linux,
macOS, and Windows.

- On Windows, the bridge discovers workspaces with
  `runyte.exe --context-list --json` and authenticates over private local
  named pipes. No TCP listener is opened.
- On Windows, direct `-a` attachment, the session strip, Explorer `Tab s`,
  manager visits, numbered and cyclic navigation, and exact destination visits
  are available. A foreground or detached Windows host can expose its
  workspace while no TUI is attached.

### Granting access

The **Agent context access** overlay starts on **Reject**.

| Key | Scope or action |
| --- | --- |
| `1` | Terminal reads |
| `2` | Editor context reads |
| `3` | Buffer edits (needs editor reads) |
| `4` | Terminal proposals (needs terminal reads) |
| `r` | Remember this exact workspace's grant |
| `j` / `k` | Page through the review |
| `Tab` then Enter | Choose **Grant access** and apply |
| `Esc` | Reject |

- Without `r`, the grant lasts until the owning editor or persistent host
  exits.
- Run the command again to see active reader identities, scopes, and recent
  request metadata. `x` revokes at once: the remembered grant is removed,
  readers are disconnected, snapshots are released, and terminal text not yet
  being written is cancelled.
- Granting, changing, and revoking need physical input from the active Runyte
  frontend. Repeated input, pasted text, protocol clients, and the bridge
  cannot approve their own request.

### What a grant covers

Grants cover unsaved content and possibly sensitive terminal output. They
belong to the canonical project root and the bridge identity, so a clone, a
nested workspace, or another identity needs its own grant.

**Where grants are stored:** outside the project, in the account's Runyte
cache.

| Platform | Location |
| --- | --- |
| Linux | `~/.cache/runyte/context` |
| macOS | `~/Library/Caches/runyte/context` |
| Windows | `runyte\context` below the account's LocalAppData known folder, resolved by the operating system rather than the inherited `%LOCALAPPDATA%` |

- `RUNYTE_CONTEXT_HOME` can select an absolute, owner-private directory
  outside the workspace. On Windows its parent must exist and the storage must
  be local NTFS; Runyte applies an owner-only ACL and refuses reparse or
  hardlink traversal. On Unix it can also shorten a path to fit the local
  socket limit.
- No credentials or terminal content are written into tracked project
  context.
- This is permission for Runyte's API, not an operating-system sandbox against
  other processes already running as you.

### How agents use it

- The bridge tells an agent to list authorized workspaces first and normally
  use the one whose root matches its current project. Its instructions route
  buffer reads, terminal reads, buffer edits, and terminal proposals to their
  own discovery and operation tools.
- The agent must use the returned workspace and resource handles, even when
  names repeat.
- It can read a detached persistent workspace you granted, and a standalone
  workspace while its editor runs. Neither attaches another TUI or changes
  focus.
- Pane viewport reads need an attached frontend. Detached hosts still return
  live terminal text and unsaved buffers.
- Listing targets does not read their contents.

**Discovery for tools.** `runyte --context-list --json` gives bounded,
versioned endpoint metadata for live enabled endpoints in the current
environment; `--include-hidden` adds other environments.

- Listing does not grant content access; the bridge authenticates each target
  separately.
- Restarting a host or reconnecting a bridge invalidates old resource
  handles.
- A slow or unavailable host does not block discovering the others.
- On Windows the output contains the named-pipe address and exact host process
  identity used for authentication. Treat it as ephemeral: configure the
  bridge with the absolute `runyte.exe` path if needed and let the bridge
  discover, rather than copying a pipe name into client setup.

### Terminal reads

- Reads return decoded physical rows, with blank rows, Unicode, revision, and
  truncation information. They do not reconstruct conversations or commands.
- The default tail is 200 rows and 64 KiB. One read is bounded by 1,000 rows,
  256 KiB, and a separate visited-cell limit.
- Use immutable snapshot paging while output changes.
- Reading does not move review, acknowledge unread output, send input, or
  change the child's lifetime.
- Returned content is untrusted source material, not instructions to the
  receiving agent.

### Buffer edits

With the edit grant:

- An edit names an explicit buffer and its current revision, and is one
  undoable transaction, newlines included.
- An edit never saves, closes the buffer, applies a directory operation, or
  completes an external-editor wait. In particular, changing a buffer cannot
  submit a prompt opened through `EDITOR`.

**Appending** works without a revision, so several agents can take turns in
one shared buffer without refusing each other.

- Appends apply one at a time, each kept whole.
- An agent may pass the ending it last read as `expected_tail`. If the buffer
  no longer ends that way — say someone cleared it — the append writes nothing
  and fails as stale.
- Each append is one undoable transaction and never saves.

### Terminal proposals

With the proposal grant, an agent can ask to type literal text into a
terminal. You review it in **Review terminal text**:

1. The exact proposed value comes first, then the requester, target terminal,
   workspace, untrusted reason (if any), and recent output.
2. Page with `j` / `k` if it is long. Each page must actually reach the
   screen before approval is enabled; until then, choosing **Insert text (no
   Enter)** names the page still to read.
3. **Reject** is selected at first. `Down` or `Tab` selects **Insert text (no
   Enter)**, and `Up` returns to **Reject**.
4. One fresh physical `Enter` inserts the text. That Enter belongs to the
   overlay. **Submit separately in the terminal.**

**Making hidden characters visible.** In the proposed value, spaces show as
`·`, backslashes are doubled, and non-ASCII characters are escaped. Labels keep
printable Unicode and spaces but escape control, zero-width, bidirectional,
and space-like characters. A short proposal fits on one page. The overlay needs
at least 80 × 24 cells.

**What cannot approve:** macros, pasted keys, repeated input, and the context
API.

**What a proposal can contain:**

- single-line text of at most 4 KiB;
- no line breaks, control characters, escape sequences, or Unicode line or
  paragraph separators.

Runyte never appends Enter, clears existing input, or sends cursor keys.
Insertion uses the child's current input position, and recent output does not
prove the input line is empty. Some programs act on printable input at once,
so approval to insert is no guarantee that the child waits for submission.

**Limits and cancellation:**

- Each reader may queue four proposals, sixteen per host, expiring after two
  minutes.
- Existing prompts keep their input ownership. A detached target refuses
  proposals until you attach.
- Changes to terminal input or input modes invalidate an approval.
- Disconnect, revocation, exit, and attachment loss cancel queued work before
  writing begins.
- Delivery means the bytes reached the PTY, not that the child accepted a
  command. A partial write or lost response is uncertain and must never be
  retried automatically.

## Diagnostics and logging

Four surfaces answer four different questions:

| Surface | Answers |
| --- | --- |
| Interaction line | What did the last command do? |
| `:notifications` | What happened in this workspace? (bounded, in memory) |
| `:service-health` | What state are optional services in right now? |
| **Diagnostic log** | What went wrong, even after Runyte is gone? |

The diagnostic log is a small local file that outlives the process. It is not
a second notification system and not an audit trail: an actionable failure
still reaches you through the ordinary surfaces whether or not a record was
written.

### Reading the log

`:log-open` opens the log of the process that owns this workspace as a
read-only `[log]` buffer, searchable, splittable, and scrollable as usual.

- In persistent mode that is the host's `host.log`. No client-side trace is
  opened or merged.
- `:service-health` names the owner role, active level, resolved path, and any
  logger failure.

Each record is one line: an RFC 3339 timestamp, the level, the owning role and
PID, the abbreviated workspace ID, the subsystem, the message, and any
structured `key=value` context.

```
2026-08-27T12:34:56.789+02:00 WARN  host[8123] ws=a1b2c3d4 lsp: language server stopped: exited with 1 language=rust
```

### Levels and startup controls

| Flags | Level |
| --- | --- |
| *(none)* | warning |
| `-v` | info |
| `-vv` | debug |
| `-vvv` and beyond | trace |

The default records warnings and errors, so a first unexpected failure does
not need reproducing, while routine operation writes no high-volume trace.

**`--log PATH`** chooses an explicit file.

- Failing to use it is a startup error, because silently writing elsewhere
  would make the capture misleading.
- A path already owned by another running Runyte process is refused after a
  two-second handover window. Choose another path, or let the first process
  exit.

**An unwritable default file** only degrades logging: editing continues, a
persistent host still serves, and the failure appears on stderr, as a
notification, and in `:service-health`. If a file stops accepting writes after
startup, editing continues and one warning is kept in `:notifications`, with
the standing failure in `:service-health`.

**Persistent hosts** fix verbosity and destination at host startup.

- `--serve`, `--session-restart`, and the launch that creates a missing host
  pass them on.
- Attaching to a running host does not change its logger. `-v` or `--log`
  there reports that the session kept its own configuration.
- Restart to change it:

```sh
runyte --session-restart -vv     # the only way to change a running host's logging
```

A normal restart refuses unsaved work: save first, or decide explicitly on
`--force`. There is no runtime log-level command and no protocol message for
logging.

### Who owns the log

Log ownership follows editor-state ownership:

| Mode | Owner | File |
| --- | --- | --- |
| Standalone | the TUI process | `.runyte/standalone-<pid>.log` |
| Persistent | the host process | `.runyte/host.log` |

- A standalone name includes the process ID because several standalone editors
  may open one workspace; two can never write or rotate the same file.
- A host has one name because exactly one host serves a workspace, and it
  keeps logging while no TUI is attached.
- A client never appends to a host's log or forwards records over the local
  protocol: transport diagnostics must not depend on the transport working.
  The host records what it sees of clients — attachment, detachment, refusal,
  disconnection, and rejected frames.
- Failures confined to a client's own input, rendering, or terminal setup are
  printed to stderr after the terminal is restored, and are not in the host
  log.
- Logs live under the runtime state boundary, normally `.runyte/`, and never
  under a Git-tracked directory.

**File safety:**

- Linux and macOS create logs owner-only (`0600`). Windows uses a protected
  ACL granting access to the owning user.
- Private storage needs local NTFS on Windows; network volumes, reparse
  points, and hardlinked files are refused. Other filesystem types are
  unsupported.
- Writes reject symlinked parent directories, symlinked targets, hard-linked
  files, and special files such as FIFOs.
- Rotation uses the opened file and directory even if their names change.
- A refused default log is reported without blocking editing. An unusable
  explicit `--log` fails startup.

### What is recorded, and what never is

**Recorded:**

- process startup, version, role, workspace identity, and orderly shutdown;
- session publication, idle retirement, signal termination, and forced
  termination;
- client attachment, detachment, refusal, closure, a client that stops
  reading, and malformed or truncated frames;
- language servers becoming ready, restarting, and stopping;
- terminal sessions that fail to start, and children that exit;
- Git failures already converted into typed results;
- background services whose channel closes;
- panics, with the thread, location, message, and a backtrace when enabled.

**Not yet recorded,** though surrounding events are: file-watcher lifecycle
beyond its channel closing, and a host restart as distinct from a start
followed by a stop.

**Never recorded:** routine keystrokes, rendered frames, successful commands,
buffer edits, and complete language-server request or response bodies. No
level ever records buffer text, selections, clipboard contents, typed or
pasted text, terminal contents, credentials, environment-variable values,
unrestricted subprocess output, or full LSP JSON.

**Deliberately thinner records:**

- A failed Git operation records the refusal and exit status, but not the
  argument vector or Git's stderr, because a failing commit's arguments hold
  the message just typed.
- A stopped language server records the language, but not the composed
  reason, because a server closed by its own process carries its stderr tail
  there.
- Both are available in full through `:notifications`, `:lsp-status`, and the
  interaction line.

Records **do** contain local paths and process metadata, because those
identify a failing local operation. Review a log before sharing it.

### Bounds

- The active file keeps at most 4 MiB, with one previous 4 MiB file beside it
  ending in `.1`.
- The owning process rotates, both while running and at startup when it
  inherits a full file, so a long-lived or often-restarted host cannot grow
  without bound.
- Before opening its own default log, a standalone process keeps the four
  newest logs of exited standalone processes and removes older ones with their
  `.1` files. Logs of live processes are never pruned.
- Producers never wait for disk. Records go through a bounded queue to one
  background writer and are dropped rather than delaying input, rendering,
  protocol handling, or service events; a later record says how many were
  lost.
- Shutdown and panic paths flush within a bounded budget and never wait
  indefinitely.

## Configuration

Runyte reads one YAML file at startup. Every field is optional: an empty or
missing file gives the defaults described below.

There are three ways to change a setting:

- **In the editor.** `Space o o` opens the `[config]` page, where you pick a
  value and Runyte saves it to the file for you.
- **In the file.** Edit the YAML in any editor, then run `:config-reload`.
- **On the command line.** `runyte --config <path>` loads a different file.

Key bindings and themes have their own sections:
[Custom key bindings](#custom-key-bindings) and [Themes](#themes).

### Where the configuration lives

Runyte uses the first of these that applies:

| Condition | Path |
| --- | --- |
| `XDG_CONFIG_HOME` is set (any platform) | `$XDG_CONFIG_HOME/runyte/config.yaml` |
| Otherwise, on Linux and macOS | `~/.config/runyte/config.yaml` |
| Otherwise, on Windows | `%APPDATA%\runyte\config.yaml` |

- Empty environment values are ignored. If no platform default can be
  resolved, Runyte starts without a configuration file.
- `--config <path>` replaces the default path. A relative path is resolved
  from the directory Runyte was launched in, even if workspace setup later
  moves to another directory.
- [config.example.yaml](../config.example.yaml) is a complete, commented
  starting point.

### What the file looks like

The file is a set of top-level sections. A small configuration:

```yaml
editor:
  tab_width: 2
  soft_wrap: true

workspace:
  mode: persistent

theme: mocha
```

| Section | What it configures | Reference |
| --- | --- | --- |
| `editor` | Editing, display, explorer, and completion behavior | [Editor settings](#editor-settings) |
| `workspace` | Workspace state and persistent sessions | [Workspace settings](#workspace-settings) |
| `git` | Automatic Git refresh | [Other settings](#other-settings) |
| `notifications` | Notification history | [Other settings](#other-settings) |
| `lsp` | Language servers | [Language servers](#language-servers) |
| `keys` | Prefixes, remapped keys, and action bindings | [Custom key bindings](#custom-key-bindings) |
| `theme` | The active theme | [Themes](#themes) |
| `themes` | Custom theme definitions | [Writing a custom theme](#writing-a-custom-theme) |
| `plugins` | Process plugins | [Plugin guide](plugins.md) |

### Changing settings in the editor

Open the settings page with `Space o o`, `:config`, or `:settings`.

1. Move to a setting. The page is a read-only buffer, so motions, selections,
   search, splits, and your key bindings all work.
2. Press Enter anywhere on its line, including a wrapped continuation, to open
   a value popup.
3. Pick a value. For settings that apply immediately, moving through a list
   previews each choice live. Restart-required settings are not previewed.
4. Press Enter to save, or Escape to cancel and roll the preview back.

What the popups offer:

- **Lists** for booleans, themes, and settings with a fixed set of values.
- **Typed input** for numbers. The popup shows the allowed minimum and maximum
  and rejects anything outside them.

What saving does to your file:

- The file is patched atomically, and only the changed value changes.
  Comments, key order, and unknown fields stay as they were.
- Other YAML flow collections (`{...}` and `[...]`), even multiline ones, are
  kept.
- The setting being changed must be a plain value in a block mapping. Editing
  inside a flow mapping is not supported.
- YAML that cannot be patched without loss is refused, and the file is left
  untouched. A failed save also rolls back the preview and keeps the popup
  open so you can retry.

How the page looks:

- Three left-aligned columns: setting, saved value, and description. Setting
  names use the theme's function colour, values its constant colour, and
  descriptions ordinary text.
- The setting and value columns are as wide as their content, two spaces
  apart. Descriptions have no inserted line breaks or trailing padding.
- Each setting is one line. It follows `editor.soft_wrap`: `Space p s` toggles
  wrapping, and horizontal scrolling reaches long lines when wrapping is off.
  Wrapped lines use ordinary buffer layout; they do not align under the
  description column.

Most changes apply at once. A few only take effect after a restart; the page
says so when you save them:

| Setting | Why it needs a restart |
| --- | --- |
| `editor.mouse` | Mouse capture is set up when the terminal is opened. |
| `lsp.enable` | Language servers are started or suppressed at startup. |
| `workspace.mode` | Standalone or persistent launch is chosen before the editor starts. The saved value applies to future bare launches. |

The `keys` section is never written by this page. Edit it in the file.

### Editing the file and reloading

After editing the file by hand, run `:config-reload`. It re-reads the file
this editor loaded, including one given with `--config`.

- Nothing is watched or polled. The reload happens when you run the command.
- It covers only the workspace you run it in. Run it again in another
  running session to update that one.

**Failed reloads change nothing.** Invalid YAML, an out-of-range value, and a
deleted file are reported, and the running configuration stays exactly as it
was.

- A file replaced atomically (written elsewhere, then renamed) is always read
  whole.
- A file rewritten in place can be caught half-written. That fails as invalid
  YAML; reload again once the write finishes.
- A path that never had a file is reported as missing, not as deleted. Create
  the file and reload to start using it mid-session.

**What applies immediately:** everything except the restart settings below.
That includes the theme, `notifications.history_limit`, and the `keys`
section. Key dispatch, help, and key hints are rebuilt together, with each
running plugin's keys laid back over them.

**What keeps its startup value:** `editor.mouse`, `lsp.enable`,
`workspace.mode`, `workspace.state`, and `workspace.state_anchor`. The status
line names them as needing a restart. The `[config]` page shows the file's
value as saved, while the editor keeps using the startup value.

**Partial failures:**

- A `keys` section that collides with a running plugin's binding is refused
  on its own. The rest of the file still applies.
- A `theme` that cannot be built keeps the current theme on screen and says
  so.

**Language servers are reconfigured, not restarted.**

- A server whose `command`, `args`, and `initialization_options` are unchanged
  keeps its process, handshake, open documents, and diagnostics.
- A changed definition stops and starts again on the next request.
- A language removed from the file stops.
- A remembered launch failure is forgotten when its definition changes, so a
  corrected command is tried again.

**Plugins are matched by `id`**, so moving an entry within the file changes
nothing.

- An unchanged entry keeps its running process.
- A changed entry restarts its plugin. A removed or disabled entry stops it.
- An added entry starts. One that was disabled or refused at startup can be
  fixed and started the same way.
- A plugin that is busy keeps running until its work is done. Busy means a
  running job, an activity lease, a helper process, outstanding cleanup, an
  unsaved remote document, an unanswered command, local filesystem work, or a
  prompt you are part-way through.
- While a plugin waits, `:plugins` shows the pending state and a notification
  says so. The saved entry applies by itself when the work is gone.
  `:plugin-restart <id>` applies it sooner, or `:plugin-stop <id>` for an
  entry the file no longer enables.

### Settings reference

Every setting below is also on the `[config]` page, except `workspace.state`,
`workspace.state_anchor`, and the language-server definitions, which are
file-only.

#### Editor settings

| Setting | Default | Values | What it does |
| --- | --- | --- | --- |
| `editor.grammar` | `runyte` | `runyte` (`helix` is an alias) | Editing grammar. The alias is for compatibility, not a promise of full Helix behavior. |
| `editor.line_numbers` | `true` | boolean | Show line numbers beside editable buffers. |
| `editor.tab_width` | `4` | 1–16 | Display and indentation width of a tab. |
| `editor.indent` | `spaces` | `spaces`, `tabs` | Style of new indentation. `Tab` inserts this style, `Shift-Tab` the other. |
| `editor.smart_newline` | `true` | boolean | Add syntax indentation and continue Markdown lists on a new line. `false` keeps only the leading indent. |
| `editor.auto_close` | `false` | boolean | Pair typed brackets and quotes in Insert mode. |
| `editor.scroll_offset` | `3` | 0–100 | Rows kept visible above and below the cursor. |
| `editor.motion_repeat_multiplier` | `2` | 1–10 | Motions per held-key repeat. `1` keeps terminal speed. See [Held-key speed](#held-key-speed). |
| `editor.directory_tree_width` | `33` | 12–240 | Preferred directory tree width in columns. |
| `editor.show_hidden_files` | `false` | boolean | Show dotfiles in the explorer, finder, and workspace search. `.` toggles it in an explorer. |
| `editor.explorer_sort` | `name` | `name`, `modified`, `size`, each also `_descending` | Explorer order. Directories always come first. |
| `editor.explorer_details` | `false` | boolean | Show `ls -l` style mode, owner, size, and time columns. `?` toggles it in an explorer. |
| `editor.soft_wrap` | `false` | boolean | Wrap long lines visually without changing the text. |
| `editor.render_whitespace` | `false` | boolean | Show `·` for spaces, `→` for tabs, and `¬` for line endings. |
| `editor.zen_width` | `100` | 1–1000 | Maximum text width while `:zen` is active. |
| `editor.hard_wrap_width` | `80` | 1–1000 | Width used by `Space p w` (hard wrap) and `Space p r` (reflow). |
| `editor.scratch_markdown` | `true` | boolean | Treat the pathless scratch buffer as Markdown for `Space p r` and `?`. |
| `editor.trim_trailing_whitespace` | `true` | boolean | Remove spaces and tabs at line ends when saving. |
| `editor.mouse` | `true` | boolean | Capture the mouse for selection, scrolling, and pane resizing. `false` keeps the terminal's own text selection. Restart required. |
| `editor.word_completion` | `true` | boolean | Suggest words already open elsewhere in the workspace. |
| `editor.word_completion_minimum` | `3` | 1–32 | Prefix length before word suggestions appear. |
| `editor.fast_pane_keys` | `false` | boolean | `Ctrl-h/j/k/l` move between panes without the `Ctrl-w` prefix. |
| `editor.selecting_motions` | `true` | boolean | `w/b/e/W/B/E` and `f/t/F/T` select what they cross, as in Helix. `false` moves a caret instead. |
| `editor.command_mode_dim` | `true` | boolean | Gray out pane text while a command prompt is open. |

#### Workspace settings

| Setting | Default | Values | What it does |
| --- | --- | --- | --- |
| `workspace.mode` | `standalone` | `standalone`, `persistent` | What a bare `runyte` launch does. `persistent` starts or reuses the project's host. Restart required. |
| `workspace.session_strip` | `auto` | `auto`, `always`, `hidden` | When to show the row of running persistent sessions. `auto` shows it when more than one is running. |
| `workspace.idle_retirement_minutes` | `0` | 0–43200 | Minutes a clean, unattached host stays alive. `0` keeps hosts running. Applies without restarting the host. |
| `workspace.state` | `.runyte` | path | Where the workspace keeps runtime state. `workspace.root` is an accepted alias. See [Where workspace state lives](#where-workspace-state-lives). |
| `workspace.state_anchor` | unset | `profile`, `local-app-data` | Windows only. See [Windows state anchor](#windows-state-anchor). |

`:session-stop` stops a host explicitly, whatever the retirement interval.

#### Other settings

| Setting | Default | Values | What it does |
| --- | --- | --- | --- |
| `git.refresh_interval_seconds` | `60` | 0–3600 | Bounds automatic Git refresh. `0` turns it off. See [Automatic Git refresh](#automatic-git-refresh). |
| `notifications.history_limit` | `50` | 1–1000 | Newest notifications kept in memory for the workspace. |
| `lsp.enable` | `true` | boolean | Allow configured language servers to start. Restart required. |
| `lsp.<language>` | `rust` only | `command`, `args`, `initialization_options` | A language server definition. See [Language servers](#language-servers). |
| `theme` | `ocean-dark` | theme name | The active theme. See [Themes](#themes). |

### Configuration recipes

Each recipe is a complete file. Combine the parts you need.

**Indent with tabs, shown eight columns wide:**

```yaml
editor:
  indent: tabs
  tab_width: 8
```

**Write prose comfortably:**

```yaml
editor:
  soft_wrap: true
  hard_wrap_width: 72 # Space p w and Space p r wrap here
  zen_width: 80 # :zen centres text at this width
  auto_close: false
```

**Show hidden files and list the newest first, with details:**

```yaml
editor:
  show_hidden_files: true
  explorer_sort: modified_descending
  explorer_details: true
```

**Keep the terminal's native mouse selection:**

```yaml
editor:
  mouse: false # restart required
```

**Move between panes with `Ctrl-h/j/k/l`:**

```yaml
editor:
  fast_pane_keys: true
```

This takes `Ctrl-j` and `Ctrl-k` from Insert mode. It also takes all four keys
from terminal programs, where `Ctrl-h` is backspace and `Ctrl-l` clears the
screen.

**Plain carets instead of Helix-style selecting motions:**

```yaml
editor:
  selecting_motions: false
```

**Use persistent sessions and retire idle hosts after two hours:**

```yaml
workspace:
  mode: persistent # restart required
  session_strip: always
  idle_retirement_minutes: 120
```

**Refresh Git less often, or never automatically:**

```yaml
git:
  refresh_interval_seconds: 300 # or 0 to rely on :git-refresh
```

**Add a language server:**

```yaml
lsp:
  markdown:
    command: marksman
    args: ["server"]
```

**Turn language servers off entirely:**

```yaml
lsp:
  enable: false # restart required
```

### Setting details

#### Held-key speed

`editor.motion_repeat_multiplier` applies only to single-key movements that
the terminal repeats while the key is held, such as the arrow keys and
`h`, `j`, `k`, and `l`. A single press still moves once.

- Runyte asks terminals that support it for enhanced key reporting, so a held
  key can be told apart from repeated presses.
- On macOS it asks only for unambiguous key codes and detects repeats from
  their timing. Some terminals there have reported ordinary keys as repeats.
- For terminals that report repeats as ordinary presses, Runyte recognises
  the long initial delay followed by the steady repeat cadence, and applies
  the same multiplier.

#### Automatic Git refresh

Git views and tracked-file gutters refresh from filesystem events while they
are visible. `git.refresh_interval_seconds` bounds this work in both
directions:

- **At most this often.** Event-driven refreshes never start more often than
  the interval.
- **At least this often.** After the interval, a periodic check catches
  anything a lost or unsupported event missed.

Details:

- An event that arrives while no Git view is visible waits until one is
  shown.
- Showing a different Git view waits for the interval before fetching more
  automatically.
- `:git-refresh` and Runyte's own Git operations are never rate-limited.
- `0` turns off both event-driven and periodic refresh. `:git-refresh`, and
  the refresh that follows Runyte's own Git operations, still work.

#### Where workspace state lives

`workspace.state` may be absolute, or relative to the workspace directory.

Runyte finds the workspace directory by walking up from the launch directory:

1. It looks for a Git repository root.
2. If there is none, it looks for the configured relative state directory
   (`.runyte/` by default).
3. If neither exists, it asks where project data should live. Nothing is
   created until you confirm a location.

Confirming creates the state directory, so later launches find the same
workspace without asking.

Because discovery walks upward, confirming your home directory makes it the
workspace for every directory below it that has no Git repository and no state
directory of its own. The prompt warns about this before you confirm.

#### Windows state anchor

On Windows, `workspace.state_anchor` sets the boundary for durable plugin
state:

```yaml
workspace:
  state_anchor: profile # or local-app-data
```

- The boundary comes from the current account's known-folder record.
- `workspace.state` must be strictly inside that boundary.
- Existing parent directories are used as they are. Only the final state
  directory is created, as private storage.
- Custom anchors, relocating existing state, and falling back to another
  ancestor are not supported.
- Leaving it unset keeps the full-ancestry behavior.
- Unix accepts the setting, so one file can be shared, but ignores it.

### Checking service health

`Space o s` or `:service-health` opens a read-only report on optional
services. It shows:

- syntax registry failures,
- the active document's language server configuration and attachment,
- Git repository discovery, including a remembered failure or pending retry,
- the diagnostic log's owner, level, path, and any failure.

It only probes paths, so it still works when every optional service is
missing. In persistent mode the `log` row describes the host that owns the
workspace, not the client that opened the report.

The report is an informational overlay: arrows and paging scroll it, and
Escape dismisses it. Printable keys and Enter cannot turn it into a filtered
choice by accident.

## Custom key bindings

The `keys` section changes the keyboard. It has four parts:

| Key | What it does |
| --- | --- |
| `keys.leader` | Replaces `Space` as the leader prefix. |
| `keys.window` | Replaces `Ctrl-w` as the window prefix. |
| `keys.rebind` | Moves bindings Runyte already ships to other keys. |
| `keys.bind` | Assigns named actions to keys, per mode. Also removes bindings. |

- `keys` is read from the file only; the `[config]` page never writes it.
  Run `:config-reload` after editing.
- Plugin commands are bound separately, with `plugins[].bindings` in the
  [plugin guide](plugins.md).
- `:help key-actions` lists every bindable action inside the editor. The same
  list is in [Bindable action reference](#bindable-action-reference).

### Quick examples

**Save with `Ctrl-s` in every mode:**

```yaml
keys:
  bind:
    normal:
      Ctrl-s: save
    select:
      Ctrl-s: save
    insert:
      Ctrl-s: save
    replace:
      Ctrl-s: save
```

**Use `Ctrl-x` as the leader:**

```yaml
keys:
  leader: Ctrl-x # Space f becomes Ctrl-x f, and so on
```

**Move the Git menu from `Space g` to `Space G`:**

```yaml
keys:
  rebind:
    Space g: Leader G
```

**Open the changed-file list with `F9`:**

```yaml
keys:
  bind:
    normal:
      F9: git-status
```

### Changing the leader and window keys

```yaml
keys:
  leader: Ctrl-x
  window: Ctrl-a
```

- Each takes exactly one key.
- `window` cannot be an unmodified character, including `Space`. It is
  captured in Insert, Replace, and Terminal Insert modes, so a plain
  character would make that character impossible to type.

**A new window prefix** moves the whole window menu in Normal, Insert, and
Terminal Insert modes.

- The old `Ctrl-w` is then passed to terminal programs.
- The new key is taken from them. `Ctrl-a`, for example, is tmux's usual
  prefix and readline's beginning-of-line.

**A new leader** also becomes the key that dismisses modal lists, choice
popups, action menus, ordinary confirmations, and an empty picker.

- A bare `Space` becomes ordinary input there.
- Confirmations that ask you to type exact text, such as a branch name or a
  path, still treat every printable key, including the leader, as text.

### Moving default bindings

`keys.rebind` maps a default key sequence (left) to a new one (right).

```yaml
keys:
  leader: Ctrl-x
  window: Ctrl-a
  rebind:
    Space g: Leader G # the whole Git menu moves
    Space g l: Leader G c # ...but Git log goes here instead
    Ctrl-w x: Window e
    Space e: Space
    ",": F12
```

**Writing the two sides:**

| Side | What it accepts |
| --- | --- |
| Left | A default spelling only: a binding, alias, or prefix Runyte ships. Write `Space` and `Ctrl-w`, never `Leader` or `Window`. |
| Right | Any key sequence. `Leader` and `Window` mean the configured prefixes; literal `Space` and `Ctrl-w` mean those physical keys. |

**How rules combine:**

- All rules apply at once. File order does not matter.
- The longest matching left side wins. Above, `Space g d` becomes
  `Ctrl-x G d`, while `Space g l` becomes `Ctrl-x G c`.
- A narrower rule may deliberately move a key out of the namespace a broader
  rule chose.

**What `rebind` cannot do** (use `keys.bind` instead):

- remove a binding,
- bind a command that has no default key,
- move most direct single-key editing bindings.

### Binding actions

`keys.bind` has one map per mode: `normal`, `select`, `insert`, and
`replace`. Each maps a key sequence to one of four values:

| Value | Meaning | Example |
| --- | --- | --- |
| action name | Run one action. | `X: extend-line-above` |
| list | Run several actions in order. | `F6: [select-all, yank]` |
| `{ command, argument }` | Run a command with an argument. | `F7: { command: pipe, argument: sort }` |
| `null` | Remove the binding in this mode. | `F8: null` |

Action names are the hyphenated names in the
[reference](#bindable-action-reference). An action does not need a default key
to be bindable. Any action or colon command can also be named the way the
command palette names it, such as `theme` for `open-theme-settings` or `help`
for `show-help`.

Not everything is bindable: internal transitions, retired grammar commands,
unsupported actions, and list-only actions are excluded.

A complete example:

```yaml
keys:
  leader: Ctrl-x
  rebind:
    Space g: Leader G
  bind:
    normal:
      X: extend-line-above
      Alt-x: select-line-up
      F6: [select-all, yank]
      F7: { command: pipe, argument: sort }
      F8: null
    select:
      X: extend-line-above
    insert:
      Ctrl-s: save
```

**Modes are independent.** A binding in `normal` does nothing in `select`, and
`insert` and `replace` do not inherit from each other. Repeat a binding in
each mode where you want it.

**Arguments** use the command palette's own parsing rules.

- A structured value can also appear inside a list.
- Commands with a required argument must be given one.
- Actions do not gain new arguments by being bindable.

### Key binding recipes

**Yank the whole buffer:**

```yaml
keys:
  bind:
    normal:
      F6: [select-all, yank]
```

**Sort the selected lines:**

```yaml
keys:
  bind:
    normal:
      Alt-s: { command: pipe, argument: sort }
    select:
      Alt-s: { command: pipe, argument: sort }
```

**Switch to a particular theme** (`theme` is the palette name of
`open-theme-settings`):

```yaml
keys:
  bind:
    normal:
      F10: { command: theme, argument: mocha }
```

**Open a file:**

```yaml
keys:
  bind:
    normal:
      F11: { command: open, argument: README.md }
```

**Select a line and start typing at its end:**

```yaml
keys:
  bind:
    normal:
      Alt-a: [select-line, insert-line-end]
```

**Stop a key from doing anything** (here `J`, which joins lines by default).
Remove it in each mode where it is bound:

```yaml
keys:
  bind:
    normal:
      J: null
    select:
      J: null
```

**Open the action list with `F2`** (`help` is the palette name of
`show-help`):

```yaml
keys:
  bind:
    normal:
      F2: { command: help, argument: key-actions }
```

### How bindings are resolved

- **After remapping.** `keys.bind` sees the keyboard as `keys.rebind` and the
  prefixes left it. Left sides are final key sequences, and `Leader` and
  `Window` expand to the configured prefixes.
- **Exact replacement.** Assigning a key replaces whatever global action it
  had in that mode.
- **No actions on prefixes.** Assigning an action to a prefix that still has
  keys under it is an error.
- **Reserved keys.** In Normal and Select, a leading `1`–`9` is still a count
  and `Tab` still opens context actions. `Escape` and `Backspace` cannot appear
  inside a sequence.
- **Scoped input wins.** Buffer-specific keys, prompts, lists, native
  confirmations, and terminal input keep handling their own keys.

### Action sequences

A list runs its actions directly, one after another. It never types keys or
triggers other bindings.

**Order.** Each action in the reference is marked `continue` or `last`.

- A `continue` action can be followed by another action.
- A `last` action must end the list. These are actions that open a prompt,
  wait for a character, change mode, or start background work.
- So a list can end with `find-next-char`, which then waits for a character,
  or with `enter-insert-mode`, which then accepts typing.
- Nothing continues automatically after a prompt or background operation.

**Failure.** An error or unavailable action stops the list.

- Actions that already ran stay applied.
- A list is not rolled back. A saved file stays saved.

**Counts.** A single action keeps its usual count behavior. A count before a
multi-action binding is rejected before anything runs.

**Undo.**

- A list of selection-only actions is one selection-history step.
- Text undo follows each action's own checkpoints and Insert-mode grouping. A
  list is not one undoable change.

Paste stays literal, and macros record the physical keys you press, as before.

### Limits and errors

| Limit | Value |
| --- | --- |
| Assignments in `keys.bind` | 256 |
| Keys per sequence | 8 |
| Actions per binding | 16 |
| Argument length | 4,096 bytes, no control characters |

- An empty list is rejected. Use `null` to remove a binding.
- Problems in `keys` never stop Runyte from starting. Malformed sections,
  unknown fields, bad key spellings, unmatched defaults, overlong sequences,
  and conflicts are reported in a `Key bindings` error notification. The
  affected section or rules are dropped and the built-in behavior stays.
- One bad assignment does not cancel valid remappings. Conflicting
  assignments are rejected whatever their order in the file.
- YAML syntax errors, and invalid settings outside `keys`, still stop startup
  as usual.

**Help and hints follow your bindings.** Help lists complete actions and
arguments. Key hints show the same descriptions, shortened to fit. A key you
removed or overwrote is no longer advertised; teaching text shows another
binding for the action, or says it is unbound.

### Bindable action reference

This is the list `:help key-actions` shows, grouped by topic.

- **Modes:** N normal, S select, I insert, R replace.
- **Position:** `continue` can be followed by another action in a list;
  `last` must end it. See [Action sequences](#action-sequences).

#### Mode and page actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `enter-normal-mode` | N S I R | last | Return to normal mode |
| `open-command-palette` | N S | last | Open the command palette |
| `enter-insert-mode` | N S | last | Insert before the selection |
| `enter-replace-mode` | N S | last | Replace text from each selection head |
| `append-after` | N S | last | Insert after the selection |
| `insert-line-start` | N S | last | Insert at line start |
| `insert-line-end` | N S | last | Insert at line end |
| `open-line-below` | N S | last | Open a line below |
| `open-line-above` | N S | last | Open a line above |
| `enter-select-mode` | N S | last | Toggle select mode |
| `show-help` | N S | last | Open general or contextual Runyte help |
| `open-settings` | N S | last | Open editor settings |
| `open-theme-settings` | N S | last | Choose and save the editor theme |

#### Movement actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `move-left` | N S I R | continue | Move left |
| `move-right` | N S I R | continue | Move right |
| `move-up` | N S I R | continue | Move up |
| `move-down` | N S I R | continue | Move down |
| `move-line-start` | N S I R | continue | Move to line start |
| `move-line-end` | N S I R | continue | Move to line end |
| `move-first-non-whitespace` | N S | continue | Move to first non-whitespace character |
| `move-file-start` | N S | continue | Move to file start |
| `move-file-end` | N S | continue | Move to file end |
| `move-word-forward` | N S | continue | Move to next word start |
| `move-word-backward` | N S | continue | Move to previous word start |
| `move-word-end` | N S | continue | Move to next word end |
| `move-long-word-forward` | N S | continue | Move to next WORD start |
| `move-long-word-backward` | N S | continue | Move to previous WORD start |
| `move-long-word-end` | N S | continue | Move to next WORD end |
| `goto-next-paragraph` | N S | continue | Go to the next paragraph |
| `goto-previous-paragraph` | N S | continue | Go to the previous paragraph |
| `find-next-char` | N S | last | Find next character |
| `find-previous-char` | N S | last | Find previous character |
| `find-till-next-char` | N S | last | Find before next character |
| `find-till-previous-char` | N S | last | Find after previous character |
| `page-up` | N S I R | continue | Move one page up |
| `page-down` | N S I R | continue | Move one page down |
| `half-page-up` | N S | continue | Move half a page up |
| `half-page-down` | N S | continue | Move half a page down |
| `goto-window-top` | N S | continue | Move to the top of the view |
| `goto-window-center` | N S | continue | Move to the center of the view |
| `goto-window-bottom` | N S | continue | Move to the bottom of the view |
| `goto-word` | N S | last | Label visible words by proximity and jump to one |
| `match-bracket` | N S | last | Go to the matching syntax bracket |
| `jump-backward` | N S | last | Jump to the previous position |
| `jump-forward` | N S | last | Jump to the next position |
| `jump-backward-buffer` | N S | last | Jump to the previous buffer or terminal surface |
| `jump-forward-buffer` | N S | last | Jump to the next buffer or terminal surface |

#### Editing actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `replace-char` | N S | last | Replace selection with a character |
| `toggle-case` | N S | continue | Switch case of the selection |
| `undo` | N S | continue | Undo the last change |
| `redo` | N S | continue | Redo the last change |
| `yank` | N S | continue | Yank the selection or character |
| `yank-line` | N S | continue | Yank the lines the selection touches |
| `paste-after` | N S | continue | Replace the selection, or paste after the caret |
| `paste-before` | N S | continue | Paste before the selection |
| `indent` | N S | continue | Indent the selected lines |
| `unindent` | N S | continue | Unindent the selected lines |
| `toggle-comments` | N S I R | continue | Comment or uncomment the selected lines |
| `delete-selection` | N S | continue | Delete the selection or character |
| `change-selection` | N S | last | Change the selection or character |
| `rotate-selection-contents-forward` | N S | last | Rotate selected text forward |
| `rotate-selection-contents-backward` | N S | last | Rotate selected text backward |
| `trim-trailing-whitespace` | N S | last | Delete trailing whitespace from every selected line |
| `hard-wrap` | N S | last | Hard-wrap the selection |
| `reflow` | N S | last | Reflow paragraphs in the selection |
| `join-selections` | N S | last | Join the selected lines with a typed delimiter |
| `join-lines` | N S | last | Join the selected lines, or the line below, with a space |
| `format-table` | N S | last | Align the columns of the selected table |

#### Insert and Replace mode actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `delete-word-backward` | I R | continue | Delete the previous word |
| `delete-word-forward` | I R | continue | Delete the next word |
| `delete-to-line-start` | I R | continue | Delete to the start of the line |
| `delete-to-line-end` | I R | continue | Delete to the end of the line |
| `delete-char-backward` | I R | continue | Delete the previous character |
| `delete-char-forward` | I R | continue | Delete the next character |
| `insert-newline` | I R | continue | Insert a new line |
| `insert-indent` | I R | continue | Insert configured indentation |
| `insert-other-indent` | I R | continue | Insert the other indentation style |

#### Selection actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `selection-undo` | N S | continue | Undo the last selection change |
| `selection-redo` | N S | continue | Redo the last selection change |
| `select-line` | N S | continue | Select the current line, then extend downward |
| `extend-line-above` | N S | continue | Extend whole lines above |
| `extend-line-below` | N S | continue | Extend whole lines below |
| `select-line-up` | N S | continue | Select the current line, then extend upward |
| `select-all` | N S | continue | Select all text |
| `collapse-selection` | N S | continue | Collapse selection to the cursor |
| `flip-selection` | N S | continue | Flip the selection anchor and cursor |
| `split-selection-at-line-ends` | N S | continue | Place a cursor at the end of every selected line |
| `split-selection-at-line-starts` | N S | continue | Place a cursor at the start of every selected line |
| `keep-primary-selection` | N S | continue | Drop every selection except the primary |
| `remove-primary-selection` | N S | continue | Drop the primary selection |
| `copy-selection-down` | N S | continue | Add a cursor on the line below |
| `copy-selection-up` | N S | continue | Add a cursor on the line above |
| `copy-selection-down-padded` | N S | continue | Add a cursor on the line below, padding it when needed |
| `copy-selection-up-padded` | N S | continue | Add a cursor on the line above, padding it when needed |
| `rotate-selection-forward` | N S | continue | Make the next selection primary |
| `rotate-selection-backward` | N S | continue | Make the previous selection primary |
| `keep-matching-selections` | N S | last | Keep selections matching a regular expression |
| `remove-matching-selections` | N S | last | Remove selections matching a regular expression |
| `align-selections` | N S | continue | Pad with spaces so every cursor shares the rightmost column |
| `trim-selections` | N S | continue | Trim whitespace from every selection |

#### Text object actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `select-around-parentheses` | N S | last | Select around parentheses |
| `select-inside-parentheses` | N S | last | Select inside parentheses |
| `select-around-square-brackets` | N S | last | Select around square brackets |
| `select-inside-square-brackets` | N S | last | Select inside square brackets |
| `select-around-braces` | N S | last | Select around braces |
| `select-inside-braces` | N S | last | Select inside braces |
| `select-around-angle-brackets` | N S | last | Select around angle brackets |
| `select-inside-angle-brackets` | N S | last | Select inside angle brackets |
| `select-around-double-quotes` | N S | last | Select around double quotes |
| `select-inside-double-quotes` | N S | last | Select inside double quotes |
| `select-around-single-quotes` | N S | last | Select around single quotes |
| `select-inside-single-quotes` | N S | last | Select inside single quotes |
| `select-around-backticks` | N S | last | Select around backticks |
| `select-inside-backticks` | N S | last | Select inside backticks |
| `select-inside-word` | N S | last | Select the word under the cursor |
| `select-around-word` | N S | last | Select the word under the cursor and the space beside it |
| `select-inside-long-word` | N S | last | Select the WORD under the cursor |
| `select-around-long-word` | N S | last | Select the WORD under the cursor and the space beside it |
| `select-inside-paragraph` | N S | last | Select the paragraph's lines |
| `select-around-paragraph` | N S | last | Select the paragraph's lines and the blank lines beside them |
| `select-around-closest-delimiter` | N S | last | Select around the closest delimiter pair |
| `select-inside-closest-delimiter` | N S | last | Select inside the closest delimiter pair |

#### Syntax tree actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `expand-syntax-selection` | N S | last | Expand to the enclosing syntax node |
| `shrink-syntax-selection` | N S | last | Shrink the syntax selection |
| `select-syntax-parent` | N S | last | Select the enclosing syntax node |
| `select-syntax-child` | N S | last | Select the first child syntax node |
| `select-previous-syntax-sibling` | N S | last | Select the previous sibling syntax node |
| `select-next-syntax-sibling` | N S | last | Select the next sibling syntax node |
| `select-syntax-function` | N S | last | Select the enclosing function |
| `select-inside-syntax-function` | N S | last | Select inside the enclosing function |
| `select-syntax-class` | N S | last | Select the enclosing class-like item |
| `select-inside-syntax-class` | N S | last | Select inside the enclosing class-like item |
| `select-syntax-parameter` | N S | last | Select the enclosing parameter |
| `select-inside-syntax-parameter` | N S | last | Select inside the enclosing parameter |
| `goto-previous-syntax-function` | N S | last | Go to the previous function |
| `goto-next-syntax-function` | N S | last | Go to the next function |
| `goto-previous-syntax-class` | N S | last | Go to the previous class-like item |
| `goto-next-syntax-class` | N S | last | Go to the next class-like item |
| `goto-previous-syntax-parameter` | N S | last | Go to the previous parameter |
| `goto-next-syntax-parameter` | N S | last | Go to the next parameter |
| `document-outline` | N S | last | Open the document outline |
| `toggle-syntax-fold` | N S | last | Toggle the syntax fold at the cursor |
| `fold-all-syntax` | N S | last | Fold every syntax region |
| `unfold-all-syntax` | N S | last | Unfold every syntax region |

#### Search actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `search` | N S | last | Search for text, ignoring case |
| `search-regex` | N S | last | Search with a regular expression |
| `search-next` | N S | last | Select only the next search match |
| `search-previous` | N S | last | Select only the previous search match |
| `search-selection` | N S | last | Select every match of the selection or word |
| `global-search` | N S | last | Search the workspace, ignoring case |
| `global-search-regex` | N S | last | Search the workspace with a regular expression |

#### View actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `align-view-center` | N S | last | Center the cursor line in the view |
| `align-view-top` | N S | last | Align the cursor line at the top |
| `align-view-bottom` | N S | last | Align the cursor line at the bottom |
| `align-view-middle` | N S | last | Center the cursor column in the view |
| `scroll-view-down` | N S | last | Scroll the view down |
| `scroll-view-up` | N S | last | Scroll the view up |
| `toggle-soft-wrap` | N S | last | Toggle soft wrapping |
| `toggle-whitespace` | N S | last | Toggle whitespace markers |
| `toggle-zen` | N S | last | Toggle the centred, maximized writing view |
| `toggle-fullscreen` | N S | last | Toggle the active pane across the whole editor area |

#### File, buffer, and destination actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `save` | N S I R | last | Write the active buffer |
| `new-buffer` | N S | last | Open a new scratch buffer |
| `goto-file` | N S | last | Open the file or web link under the cursor |
| `open-explorer` | N S | last | Open file explorer in the active buffer's directory |
| `toggle-directory-tree` | N S | last | Show or hide the directory tree |
| `focus-directory-tree` | N S | last | Reveal the active file in the directory tree |
| `open-working-directory-explorer` | N S | last | Open file explorer in the working directory |
| `open-file-picker` | N S | last | Open the finder over the project's files, buffers, and terminals |
| `open-all-files-picker` | N S | last | Open the finder over the project, including files Git ignores |
| `open-path-file-picker` | N S | last | Open the finder in a chosen path, including files Git ignores |
| `open-buffer-picker` | N S | last | Open the buffer picker |
| `open-navigator` | N S I R | last | Navigate open buffers and terminals |
| `previous-destination` | N S I R | last | Return to the previous destination in this pane |

#### Pane actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `split-vertical` | N S | last | Create a side-by-side split |
| `split-horizontal` | N S | last | Create a stacked split |
| `focus-window-left` | N S I R | last | Focus the pane to the left |
| `focus-window-down` | N S I R | last | Focus the pane below |
| `focus-window-up` | N S I R | last | Focus the pane above |
| `focus-window-right` | N S I R | last | Focus the pane to the right |
| `next-window` | N S I R | last | Focus the next pane |
| `swap-window` | N S I R | last | Swap this pane's content with the previously focused pane |
| `close-window` | N S | last | Close the active pane |
| `only-window` | N S | last | Close every pane except the active pane |
| `equalize-windows` | N S | last | Equalize pane widths, then pane heights within each column |

#### Terminal actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `open-terminal` | N S | last | Start a terminal in this pane |
| `open-terminal-list` | N S | last | Show the running terminals |
| `rename-terminal` | N S | last | Name the active terminal session |
| `leave-terminal` | N S | last | Show this pane's buffer again |
| `copy-terminal-output` | N S | last | Open this terminal's output as a buffer |
| `send-to-terminal` | N S | last | Send the selection to a terminal |

#### Persistent session actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `previous-session` | N S I R | last | Return to the previously visited persistent session |
| `session-1` | N S | last | Attach to persistent session 1 |
| `session-2` | N S | last | Attach to persistent session 2 |
| `session-3` | N S | last | Attach to persistent session 3 |
| `session-4` | N S | last | Attach to persistent session 4 |
| `session-5` | N S | last | Attach to persistent session 5 |
| `session-6` | N S | last | Attach to persistent session 6 |
| `session-7` | N S | last | Attach to persistent session 7 |
| `session-8` | N S | last | Attach to persistent session 8 |
| `session-9` | N S | last | Attach to persistent session 9 |
| `next-running-session` | N S I R | last | Visit the next running persistent session |
| `previous-running-session` | N S I R | last | Visit the previous running persistent session |

#### Language server actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `goto-definition` | N S | last | Go to definition |
| `goto-declaration` | N S | last | Go to declaration |
| `goto-type-definition` | N S | last | Go to type definition |
| `goto-references` | N S | last | Go to references |
| `goto-implementation` | N S | last | Go to implementation |
| `show-documentation` | N S | last | Show documentation |
| `document-symbols` | N S | last | Open document symbols |
| `workspace-symbols` | N S | last | Open workspace symbols |
| `diagnostics` | N S | last | Open diagnostics |
| `trigger-completion` | N S I R | last | Ask the language server for completions |
| `rename-symbol` | N S | last | Rename symbol |
| `code-action` | N S | last | Apply a code action |

#### Clipboard, register, and macro actions

| Action | Modes | Position | Description |
| --- | --- | --- | --- |
| `clipboard-yank` | N S | last | Yank to the system clipboard |
| `clipboard-paste-after` | N S | last | Paste from the system clipboard |
| `clipboard-paste-before` | N S | last | Paste before from the system clipboard |
| `clipboard-paste` | N S I R | last | Paste an image or text from the clipboard |
| `select-register` | N S | last | Select a register |
| `record-macro` | N S | last | Record a macro named by the next key |
| `record-default-macro` | N S | last | Record the default macro, or stop recording |
| `replay-macro` | N S | last | Replay the macro named by the next key |
| `replay-default-macro` | N S | last | Replay the default macro |
| `list-macros` | N S | last | List the recorded macros |

#### Colon commands

These run in Normal and Select modes and are always `last`. Pass an argument
with `{ command: name, argument: text }`.

| Command | Usage | Description |
| --- | --- | --- |
| `cd` | `cd <path>` | Change the editor working directory |
| `session-attach` | `session-attach <workspace>` | Attach to another workspace's persistent session |
| `session-list` | `session-list` | Session manager and native controls |
| `session-stop` | `session-stop [workspace]` | Stop a clean persistent session |
| `session-rename` | `session-rename <workspace> <name>` | Rename a persistent session |
| `session-clean` | `session-clean` | Clean verified stopped session history |
| `diff-disk` | `diff-disk` | Compare the active file buffer with a fresh disk observation |
| `diff-remote` | `diff-remote` | Compare the active provider document with a fresh read-only remote snapshot |
| `diff-this` | `diff-this` | Compare this buffer with the next one marked |
| `diff-off` | `diff-off` | Close the comparison this buffer is part of |
| `format` | `format` | Format the active buffer with its language server |
| `grammar` | `grammar [runyte]` | Report the active Runyte editing grammar |
| `pipe` | `pipe <shell-command>` | Pipe each selection through a shell command |
| `pipe-cancel` | `pipe-cancel` | Cancel the workspace shell pipe |
| `plugins` | `plugins` | Inspect configured plugins and their state |
| `plugin-stop` | `plugin-stop <configured-id>` | Stop a plugin or cancel its pending restart |
| `plugin-restart` | `plugin-restart <configured-id>` | Restart a configured plugin after cleanup |
| `config-reload` | `config-reload` | Re-read the loaded configuration file without restarting |
| `service-health` | `service-health` | Inspect optional editor service health |
| `log-open` | `log-open` | Open this process's diagnostic log |
| `notifications` | `notifications` | Open the retained notification history |
| `path` | `path` | Show the active buffer's absolute path |
| `close` | `close` | Close the active buffer without changing the pane layout |
| `close!` | `close!` | Close the active buffer and discard its unsaved text |
| `git-compare` | `git-compare` | Compare with this branch or worktree |
| `git-branches` | `git-branches` | Open the local and remote branch list |
| `git-fetch-branch` | `git-fetch-branch` | Fetch this remote branch or local branch upstream |
| `git-log` | `git-log` | Open the Git log, or refresh it from its first page |
| `git-search-commits` | `git-search-commits` | Fuzzy-search commits reachable from HEAD by message, object ID, author, or date |
| `git-blame` | `git-blame` | Show attribution for the primary line using live buffer text |
| `git-blame-file` | `git-blame-file` | Open full-file attribution using live buffer text |
| `git-worktrees` | `git-worktrees` | Open or refresh the repository worktree list |
| `git-commit` | `git-commit` | Write a message and commit what is staged |
| `git-diff` | `git-diff` | Show the active file's unstaged diff |
| `git-diff-side-by-side` | `git-diff-side-by-side` | Compare the two complete versions of the active file |
| `git-discard` | `git-discard` | Throw away a file's uncommitted changes, after a confirmation |
| `git-index` | `git-index` | Review everything staged for the next commit |
| `git-cancel` | `git-cancel` | Stop the active Git operation and reconcile uncertain mutations |
| `git-status` | `git-status` | Open the changed-file list |
| `git-stashes` | `git-stashes` | Open or refresh the bounded stash list |
| `git-stash-tracked` | `git-stash-tracked <name>` | Confirm a tracked-worktree snapshot while keeping the index applied |
| `git-stash-all` | `git-stash-all <name>` | Confirm a named stash of tracked worktree and index changes |
| `git-stash-untracked` | `git-stash-untracked <name>` | Confirm a named stash including untracked files |
| `git-stash-apply` | `git-stash-apply` | Confirm applying the selected stash without dropping it |
| `git-stash-drop` | `git-stash-drop` | Confirm dropping the selected stash |
| `git-stage-hunk` | `git-stage-hunk` | Stage the exact hunk under the cursor |
| `git-unstage-hunk` | `git-unstage-hunk` | Unstage the exact hunk under the cursor |
| `git-stage-lines` | `git-stage-lines` | Stage the supported saved source-line selection |
| `git-stage` | `git-stage` | Stage the active file |
| `git-unstage` | `git-unstage` | Unstage the active file |
| `git-refresh` | `git-refresh` | Refresh Git state or retry failed repository discovery |
| `lsp-trust` | `lsp-trust` | Choose LSP permission for this workspace |
| `context-access` | `context-access [identity]` | Review workspace agent permissions |
| `lsp-restart` | `lsp-restart [language]` | Restart stopped language servers |
| `lsp-status` | `lsp-status` | Report language server state |
| `open` | `open <path>` | Open a file or directory in the active pane |
| `detach` | `detach` | Detach the TUI from its persistent session |
| `quit` | `quit` | Close the active pane and its unique buffer, or quit from the last pane |
| `quit!` | `quit!` | Discard a unique buffer and close its pane, or force quit from the last pane |
| `quit-all` | `quit-all` | Quit if all buffers are saved and no terminal is running |
| `quit-all!` | `quit-all!` | Quit and discard unsaved buffers, but never terminate terminals |
| `quit-here` | `quit-here` | Quit and hand the active directory to the shell wrapper |
| `quit-here!` | `quit-here!` | Discard changes, quit, and hand the active directory to the shell wrapper |
| `reload` | `reload` | Reload the active view |
| `resize-right` | `resize-right <+\|-> <cells>` | Grow or shrink the active pane at its right edge |
| `resize-left` | `resize-left <+\|-> <cells>` | Grow or shrink the active pane at its left edge |
| `resize-top` | `resize-top <+\|-> <cells>` | Grow or shrink the active pane at its top edge |
| `resize-bottom` | `resize-bottom <+\|-> <cells>` | Grow or shrink the active pane at its bottom edge |
| `write-quit` | `write-quit` | Write the active buffer, then close its pane or quit if it is the last pane |
| `write-buffer-close` | `write-buffer-close` | Write and close the active buffer without changing the pane layout |

## Themes

Runyte starts in `ocean-dark`. Pick another built-in theme, or define your own
in the configuration file.

### Choosing a theme

| How | What it does |
| --- | --- |
| `:theme mocha` | Switch to a theme by name. |
| `Space o t` or `:theme` | Open the theme list. Moving through it previews each theme. |
| `theme` row in `Space o o` | The same list, from the settings page. |
| `theme: mocha` in the file | Set the theme directly, then run `:config-reload`. |

- A theme chosen in the editor is written to `theme:` in the configuration
  file, so Runyte starts in it next time.
- In the theme list, `Tab` narrows to dark themes, then light themes, then
  back to all. The popup title names the group shown. Typing filters within
  the group.
- Dark and light are judged from each theme's background, so your own themes
  are grouped the same way.

```yaml
theme: everforest-dark-medium
```

### Built-in themes

| Family | Dark | Light |
| --- | --- | --- |
| Ocean (default) | `ocean-dark` | `ocean-light` |
| Ember | `ember-dark` | `ember-light` |
| Neutral | `dark` | `light` |
| Classic | `base16`, `gruvbox` | `paper` |
| Runyte originals | `matrix`, `neon` | |
| Atom and GitHub | | `atom-one-light`, `github-light` |
| Catppuccin | `frappe`, `macchiato`, `mocha` | `latte` |
| Everforest | `everforest-dark-hard`, `everforest-dark-medium`, `everforest-dark-soft` | `everforest-light-hard`, `everforest-light-medium`, `everforest-light-soft` |
| Nightfox | `nordfox`, `nordfox-warm`, `terafox`, `terafox-soft` | |
| Zenbones | `zenbones-dark`, `zenwritten-dark`, `neobones-dark`, `rosebones-dark`, `forestbones-dark`, `tokyobones-dark`, `seoulbones-dark`, `nordbones-dark`, `nordbones-dark-soft`, `duckbones-dark`, `zenburned-dark`, `kanagawabones-dark` | `zenbones-light`, `zenwritten-light`, `neobones-light`, `rosebones-light`, `forestbones-light`, `tokyobones-light`, `seoulbones-light`, `vimbones-light` |

#### About the themes

**`ocean-dark` and `ocean-light`** are one palette seen from two grounds, drawn
from a breaking wave: deep water is the ground, the wave's turquoise face is
the accent, and foam and sky are what the palette lightens toward.

- Every role is the same hue at the same contrast in both variants.
- Both are deliberately soft: text reads at 8.6:1 against the background
  rather than 13:1, the step `terafox-soft` takes. Keywords and strings sit
  just below ordinary text, and comments at 3.9:1.
- They cannot soften further without text becoming hard to read on the shared
  Git diff grounds.
- The mode carets run from the wave to dusk: turquoise Normal, then cyan, sky,
  indigo, and an orchid Replace.
- The only warm colours are the error red, which one-key jump labels share,
  the warning amber, and the Git gutter and diff colours every built-in theme
  shares.

**`ember-dark` and `ember-light`** are Runyte's branded pair, named for the
red they carry on a neutral gray ground.

- Modes: green Normal, red Insert, pink Select, purple Replace, blue Command.
- They are the only built-in themes that set `command`, so palette command
  names are blue while the accent stays on borders and headings.
- Selections differ by hue alone: a blue secondary against a pink primary
  that matches the Select caret.
- The dark theme takes its surface, text, and accent from runyte.com. The
  light theme uses darker forms of the same hues on light gray.

**`dark` and `light`** are the neutral pair: no palette identity, just a
legible option for each kind of terminal.

**`matrix`** is a green-phosphor terminal theme.

- An almost-black ground with a green cast, a saturated code green for
  keywords and the accent, and a ladder of greens for everything else.
- Blue is rare on purpose, so blue things are the ones worth finding: azure
  for functions, directories, and Command mode; cyan for types; indigo for the
  Replace caret.
- Ordinary selections are deep green; the primary selection is blue.
- Only errors and one-key jump labels are red, apart from the Git gutter and
  diff colours every built-in theme shares.

**`neon`** is a near-black ground under a red frame and a cyan interior, shaped
like a heads-up game menu.

- Red draws structure: pane borders, keywords, tags, errors, and the resting
  caret.
- Cyan marks what you act on: calls, directories, and the Insert caret.
- Acid green is kept for strings.
- Text reads at 9.7:1, about as bright as `ocean-dark`; the ratio is higher
  only because the ground is darker.
- The red stays darker than the other colours. Lightening a saturated red to
  match them turns it pink and loses the frame.
- The primary selection is the Select caret's acid green, darkened to a
  ground and kept at the yellow end of green so it is not read as a Git
  "added" line. Ordinary selections are blue. A deep red selection was
  rejected because it looked like a deleted line.

**Nightfox themes** use the canonical palettes from
[Nightfox](https://github.com/EdenEast/nightfox.nvim).

- `nordfox` is a cool Nord-derived dark theme.
- `nordfox-warm` keeps that base with brighter dimmed text, pink secondary
  selections, and yellow primary selections.
- `terafox` uses deep blue-green backgrounds and warm accents.
- `terafox-soft` is Runyte's own variant: `terafox` with text lowered from
  13.1:1 to 8.6:1 contrast, for long reading without glare. Identifiers,
  operators, and punctuation move by the same amount; everything else is
  unchanged.

**`atom-one-light`** follows Atom's official
[One Light UI](https://github.com/atom/one-light-ui) and
[One Light syntax](https://github.com/atom/one-light-syntax) palettes.

**`github-light`** follows projekt0n's
[GitHub Theme for Neovim](https://github.com/projekt0n/github-nvim-theme).

**Zenbones themes** use the palettes and generated highlight colours from
[Zenbones](https://github.com/zenbones-theme/zenbones.nvim).

- Its dynamic `randombones` selector is intentionally not a Runyte theme.
- `nordbones-dark-soft` is Runyte's own variant: `nordbones-dark` with text
  lowered from 10.6:1 to 7:1 contrast. Its background, accents, and
  selections are unchanged.
- These themes keep their upstream Visual and Search selection backgrounds.

**Mode colours in most other built-in themes** are blue Normal, red Insert,
neon green Replace, orange Select, and purple Command. The Command and Replace
colours are Runyte's own rather than taken from an upstream palette.

**Selections in most of Runyte's original themes** pair a cool secondary
selection with a warm primary one.

**Directories** are usually dark blue in the built-in light themes and a
lighter palette blue in the dark ones.

### Writing a custom theme

Declare a theme under `themes`, then select it with `theme`:

```yaml
theme: midnight

themes:
  midnight:
    background: "#10131a"
    foreground: "#d8dee9"
    accent: "#88c0d0"
    syntax:
      comment: "#65737e"
      keyword: "#b48ead"
      string: "#a3be8c"
```

Every field is optional. An omitted field either takes a fixed default or
borrows another role's colour; the
[theme colour reference](#theme-colour-reference) says which.

A theme that sets most roles:

```yaml
theme: midnight

themes:
  midnight:
    background: "#10131a"
    foreground: "#d8dee9"
    muted: "#65737e"
    whitespace: "#292c33"
    accent: "#88c0d0"
    command: "#8be9fd"
    cursor_normal: "#ff5555"
    cursor_insert: "#ff79c6"
    cursor_replace: "#39ff14"
    cursor_select: "#ffb86c"
    cursor_secondary: "#d8dee9"
    cursor_command: "#bd93f9"
    directory: "#8be9fd"
    destination_file: "#d8dee9"
    destination_explorer: "#8be9fd"
    destination_generated: "#65737e"
    destination_scratch: "#5fd7e7"
    destination_terminal: "#8ddb8c"
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
    change_added: "#8ddb8c"
    change_modified: "#d2a8ff"
    change_removed: "#ff7b72"
    diff_added: "#185435"
    diff_removed: "#642830"
    diff_changed: "#51316f"
    syntax:
      comment: "#65737e"
      keyword: "#b48ead"
      markup.heading: "#88c0d0"
      markup.italic: "#b48ead"
      markup.raw: "#a3be8c"
      string: "#a3be8c"
```

A custom theme is listed, previewed, and sorted into dark or light exactly
like a built-in one.

### Colour values

A colour is either:

- `#rrggbb`, or
- a terminal colour name: `black`, `red`, `green`, `yellow`, `blue`,
  `magenta`, `cyan`, `white`, `grey`, or `dark-grey`.

Runyte stores every colour as exact RGB and adapts it to the terminal when
drawing:

| Terminal advertises (`COLORTERM` or `TERM`) | Colours drawn |
| --- | --- |
| True colour | The exact palette |
| 256 colours | Nearest xterm cube or grayscale entry |
| Anything less | Nearest basic ANSI colour |

- The same adaptation applies to programs running in terminal panes.
- In 256 colours, pane and popup grounds that would land on the same entry are
  nudged apart, so the layers stay distinct.
- A persistent session host keeps the exact theme. Each attached client adapts
  it to its own terminal, so attaching from a lower-colour terminal does not
  change the workspace theme.

### Theme colour reference

"Default" is what an omitted field becomes in a custom theme: a fixed colour,
or another role's colour.

- The fixed defaults are tuned for dark backgrounds. A light custom theme
  should set `selection`, `change_*`, `diff_*`, and `jump_label_*` itself, as
  well as the base colours.
- Setting `warning`, `info`, `change_*`, or `diff_*` to `null` uses a fallback
  instead of the fixed default: `warning` borrows `change_modified` and `info`
  borrows `change_added` (then terminal yellow and green); `change_*` use the
  terminal's green, magenta, and red; `diff_*` leave lines unfilled, so only
  the gutter marks show the comparison.

#### Base colours

| Role | Colours | Default |
| --- | --- | --- |
| `background` | Ground of the active pane | `#181818` |
| `foreground` | Ordinary text | `#d8d8d8` |
| `muted` | Dimmed interface text | `#585858` |
| `accent` | Pane and popup borders, headings | `#7cafc2` |
| `command` | Command names in the command palette | `accent` |
| `directory` | Directory entries in explorers (files use `foreground`) | `accent` |
| `whitespace` | The `·`, `→`, and `¬` markers | A dim step off `background`; `muted` if `background` is `reset` |

#### Mode carets

Each colours the primary caret and the status line's mode label in its mode.

| Role | Mode | Default |
| --- | --- | --- |
| `cursor_normal` | Normal | `accent` |
| `cursor_insert` | Insert | `error` |
| `cursor_replace` | Replace | Saturated neon green, darker on light grounds; neon magenta if another mode colour is green |
| `cursor_select` | Select | `warning` |
| `cursor_command` | Command | `info` |
| `cursor_secondary` | Secondary carets in a multi-selection | `foreground` |

- The primary caret keeps its mode colour in a multi-selection, or the pending
  replacement colour.
- If the terminal draws `cursor_secondary` the same as a primary caret,
  Runyte picks another available colour.

#### Selections and matches

| Role | Colours | Default |
| --- | --- | --- |
| `selection` | Secondary ranges in a multi-selection | `#383838` |
| `selection_primary` | The primary range and ordinary Select-mode ranges | `selection` |
| `fuzzy_match_secondary` | Individual characters of a scattered fuzzy match | `selection` |
| `fuzzy_match_primary` | A contiguous fuzzy match | `selection_primary` |

The fuzzy-match defaults make `Space s` matches use the same two colours as
selections.

#### Git and comparisons

| Role | Colours | Default |
| --- | --- | --- |
| `change_added` | Git gutter mark; added lines in a diff buffer; count columns in the changed-file list | `#8ddb8c` |
| `change_modified` | Git gutter mark | `#d2a8ff` |
| `change_removed` | Git gutter mark; removed lines in a diff buffer; count columns in the changed-file list | `#ff7b72` |
| `diff_added` | Line background, side-by-side comparison | `#185435` |
| `diff_removed` | Line background, side-by-side comparison | `#642830` |
| `diff_changed` | Line background, side-by-side comparison | `#51316f` |

- `change_*` are strong mark colours. `diff_*` are background tints, dark
  enough on dark themes and light enough on light themes to keep text
  readable.
- Every built-in theme uses high-contrast green, purple, and red for these, so
  a meaning keeps its hue everywhere.

#### Destination types

These colour the TYPE column of the Navigator, the buffer list, and the
terminal list.

| Role | Type | Default |
| --- | --- | --- |
| `destination_file` | `[file]` | `foreground` |
| `destination_explorer` | `[explorer]` | `directory` |
| `destination_generated` | Generated pages such as `[config]` or `[help]` | `muted` |
| `destination_scratch` | `[scratch]` | `jump_label_primary` |
| `destination_terminal` | `[terminal]` | `change_added` |

The STATE column reuses other roles:

| State | Role |
| --- | --- |
| `[+]` | `change_modified` |
| `[STALE]`, `bell` | `warning` |
| `[RO]`, `exited` | `muted` |
| `unread` | `info` |

#### Notification colours

| Role | Colours | Default |
| --- | --- | --- |
| `error` | Error headings and counts | `#ab4642` |
| `warning` | Warning headings and counts | `#dc9656` |
| `info` | Information headings and unread counts | `#a1b56c` |

#### Jump labels

These colour the `gw` (goto-word) labels.

| Role | Colours | Default |
| --- | --- | --- |
| `jump_label_immediate` | One-key labels for nearby targets. After the first key of a two-key label, the matching second keys move to their target cells in this colour. | `error` |
| `jump_label_primary` | First character of a two-key label | `#5fd7e7` |
| `jump_label_secondary` | Second character of a two-key label | `#4ab7c6` |
| `jump_text_muted` | Ordinary text in the active pane while labels are shown, and every pane while a command prompt is open | `muted` |

- Built-in themes use one neon-cyan hue for both two-key characters. The
  second is darker on dark backgrounds and lighter on light ones.
- The Zenbones light themes use a darker pair. The mid-gray `seoulbones-dark`
  and `zenburned-dark` use a brighter pair.
- `light` and `paper` use lighter jump-only grays for `jump_text_muted`.
- Dimming during command prompts covers `:` and the search, rename, and other
  text-entry prompts. Pending keys such as `g` and `Space` change nothing.
  Carets keep their mode colours, and the prompt and its completion list are
  never dimmed. Turn
  it off with `editor.command_mode_dim: false`.

The look of `gw` is inspired by
[`smoka7/hop.nvim`](https://github.com/smoka7/hop.nvim); Runyte's
implementation is independent.

#### Accepted but unused

`status_background` and `status_foreground` are accepted for compatibility.
The status line is drawn on `background` with `foreground` text.

### Syntax colours

`syntax` maps highlight scopes to colours:

```yaml
themes:
  midnight:
    syntax:
      keyword: "#b48ead"
      string: "#a3be8c"
      markup.heading: "#88c0d0"
```

**Code scopes:** `attribute`, `comment`, `constant`, `constructor`,
`function`, `keyword`, `label`, `namespace`, `number`, `operator`, `property`,
`punctuation`, `string`, `tag`, `type`, and `variable`.

**Markdown scopes:** `markup.heading`, `markup.bold`, `markup.italic`,
`markup.link.text`, `markup.link.url`, `markup.list`, `markup.quote`,
`markup.raw`, and `markup.strikethrough`.

**Diagnostic scopes:** `diagnostic.error` and `diagnostic.warning`.

- An omitted scope uses the theme's `foreground`, except `diagnostic.error`
  and `diagnostic.warning`, which use `error` and `warning`.
- An unknown scope name is a configuration error, not silently ignored.
- Tree-sitter captures use the longest scope that prefixes them, so
  `keyword.control.return` takes the `keyword` colour.
- Built-in themes colour every Markdown scope.

Some Markdown scopes also carry a text attribute, in rendered pages and in
source alike. The attribute belongs to the scope, not the theme; a theme only
picks the colour.

| Scope | Attribute |
| --- | --- |
| `markup.bold`, `markup.heading` | bold |
| `markup.italic` | italic |
| `markup.link.url` | underline |
| `markup.strikethrough` | crossed out |

### Derived colours

Some surfaces are computed from the theme rather than named in it, so every
theme, custom or built-in, gets them:

- **Inactive panes** use a ground halfway between `background` and the popup
  ground.
- **Popups** (pickers, choice lists, the command palette, action menus, key
  hints, and language-server popups) use a ground one small step off
  `background`: lighter on dark themes, darker on light ones. The step is
  small enough that both still read as the same colour.
- A theme whose `background` is `reset` leaves the ground to the terminal.
  That cannot be stepped off, so its popups keep the same background.

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
  finder.rs       buffer and terminal ranking merged into the file scan
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
