# Runyte

[![CI](https://github.com/runyte/runyte/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/runyte/runyte/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-%E2%89%A589%25-brightgreen)](context/reference/test-coverage.md)

https://github.com/user-attachments/assets/cc77a90c-25e5-4b15-a1c9-f5da7f3f12fb

*[Watch the 60-second demo](https://runyte.com/videos/runyte-demo.mp4): from a Markdown prompt to Rust code, then find text across files and terminals.*

**Runyte** is a fast modal text editor for focused work. Everything is accessed
through keybindings. No need to remember them though - every keybinding sequence
shows hints with possible completions.

Runyte can do quite a lot:
- multipane text editing
- terminal multiplexing
- file browsing and management
- Git
- fuzzy finding across all files, terminals, and buffers
- plugins written in any language
- LSP support (code analysis, formatting, jumping between functions and variables)
- Tree-sitter support for 31 languages
- optional persistent mode (attach/detach while keeping all terminals running and unsaved buffers waiting)
- word completion based on text from all open buffers
- smart file path completion (searching from current buffer and from project dir)

It also has some features making it more pleasant to work with AI agents.
The following features help you manage multiple agents and keep up with their outputs:
- Git worktree support and fast switching
- optional [MCP bridge](bridges/runyte-context/README.md) for sharing buffer and terminal contents with agents
- `Ctrl-g` in **Claude Code** or **Codex** attaches to the parent Runyte instance, instead of running a new one
  (after [setting `EDITOR`](#post-install-setup))
- **Markdown formatting** with `?`, including wide tables
- **Image pasting** with `Ctrl-v` or `Alt-v`
- Jump to any link with `gf` - works with file paths, Markdown links, web links (opens in a browser)

Having all of these features in a single binary enables strong code optimization,
consistent keybindings, consistent themes, and minimum configuration.

Runyte runs on Linux, macOS, and Windows 11.

| CPU    | System     | Automated tests | Prebuilt release | Hands-on use |
|--------|------------|:---------------:|:----------------:|:------------:|
| x86-64 | Linux      | ✅ | ✅ | ✅ |
| ARM64  | Linux      | ❌ | ✅ | ❌ |
| x86-64 | macOS      | ❌ | ✅ | ❌ |
| ARM64  | macOS      | ✅ | ✅ | ✅ |
| x86-64 | Windows 11 | ✅ | ✅ | ✅ |
| ARM64  | Windows 11 | ❌ | ❌ | ❌ |

This project is well tested. Test coverage is about 92% of lines, and CI requires at least 89%.

Every prebuilt release is built natively on its own platform and checked to start.

A few features are missing or work differently on Windows; see
[Windows support](docs/user-guide.md#windows-support).

Website: [runyte.com](https://runyte.com) ·
Documentation: [user guide](docs/user-guide.md) ·
Changelog: [GitHub Releases](https://github.com/runyte/runyte/releases) ·
Community: [r/runyte](https://www.reddit.com/r/runyte/)

## Screenshots

![Runyte displaying a Markdown document as a formatted page.](https://runyte.com/images/screenshots/rendered-markdown.webp?v=28c285c399fe)

*Read Markdown as a page. Return to source. · nordbones-dark-soft*

![Runyte Navigator listing files, an explorer, About, and running terminals, with a live terminal preview.](https://runyte.com/images/screenshots/navigator.webp?v=d0434c6aa20a)

*Jump between open buffers and terminals. · terafox-soft*

![Runyte comparing indexed and working-tree Rust source with highlighted changes in rosebones-dark.](https://runyte.com/images/screenshots/side-by-side-diff.webp?v=e74adc8b6b70)

*Compare changes side by side. · rosebones-dark*

![Claude Code and OpenAI Codex in adjacent Runyte terminal panes in frappe.](https://runyte.com/images/screenshots/coding-agents.webp?v=75cab6417c7e)

*Claude and Codex. Two terminals, one workspace. · frappe*

More examples are on the [screenshots page](https://runyte.com/screenshots/).

## Workspaces, panes, and navigation

When you start Runyte in a directory, this directory becomes your **workspace**.
All panes, open buffers, and terminal sessions belong to this workspace.

You can start a process in a terminal pane, then hide it and use that
pane for file editing. Use the **Navigator** (`Space n`) to browse open files
and terminals. You can also use the fuzzy **Finder** (`Space f`) to search
across the entire workspace including files, buffers, and terminals.

```text
Workspace
|
+-- Pane 1 ---- shows ----> Buffer A: src/main.rs
+-- Pane 2 ---- shows ----> Buffer A: same text, another view
+-- Pane 3 ---- shows ----> Terminal: shell, build, or coding agent
|
+-- Other open buffers and terminals, ready to switch to
```

Use `Ctrl-w ...` commands to manage panes.

## Terminals

Type `:terminal` (or `:t`), or press `Space t n` or `Ctrl-w t`, to start a
terminal in the current pane. A new terminal starts in Insert mode, where keys
go to the running program. Press `Ctrl-\` to switch to Normal mode: the output
keeps running, but keys go to Runyte. Press `Ctrl-\` again to review the
output. This freezes a snapshot of the output, which you can move around,
search with `s` or `/`, and copy with `y`. Press `i` to go back to Insert mode.

## Standalone and persistent modes

By default Runyte runs in standalone mode. When you quit, you stop its process.
The standalone mode is suitable for quick file edits or when you want to work
in a single workspace (directory).

Runyte also supports persistent mode in which you can attach/detach from
your session or quickly switch to other workspaces. This mode is for you
if you want to work on multiple projects or Git worktrees simultaneously.
A local **host** keeps the workspace alive while a **client** provides the
terminal interface.

Start Runyte in persistent mode with:

```sh
runyte -a  # attach to a session or start a new one
```

There are multiple ways to create a new session when you are already in Runyte:
- `Space Space` to open the session manager
- open the file explorer (`Space e`), navigate to another dir with `-` (go up) and `Enter` (go into) and press `Tab s` to start a new session in that dir
- open the integrated terminal, navigate to another dir and type `runyte -a` again

Switch between sessions with `Space Space`, or with `Shift-Left` and `Shift-Right`.

## Plugins

Official Runyte plugins are in development:

- [**ru-time**](https://github.com/runyte/ru-time) — a task list and time tracker with task notes.
- [**ru-dbviewer**](https://github.com/runyte/ru-dbviewer) — browse SQLite and PostgreSQL databases and run SQL from editor buffers.

Plugins can be written in **any programming language**. Each plugin is a
separate program you enable. It talks to Runyte in JSON over stdin/stdout.

Plugin commands start with `::` so they don't clash with built-in `:` commands,
for example `::time` in ru-time.

The repository includes examples to build on:

- [Uppercase selections](docs/plugins/uppercase.py), a minimal Python plugin.
- [Todo lists in Python, Rust, and C](docs/plugins/todo/README.md), plus an
  independent [JavaScript example](docs/plugins/tasks.mjs).
- A [local file manager](docs/plugins/applications.md#local-file-manager),
  [SFTP and FTP/FTPS browsers](docs/plugins/applications.md#sftp-browser-and-editor),
  and a [local media controller](docs/plugins/applications.md#local-media-controller).

The stable plugin contract starts with Runyte 0.3.0. Start with the [installation and protocol guide](docs/plugins.md)
or the [application authoring guide](docs/plugins/authoring.md).

## Installation

### Easy install

Install or update to the latest release on Linux or macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/runyte/runyte/main/install.sh | sh
```

The [installer](install.sh) selects the x86-64 or ARM64 archive, verifies its
SHA-256 checksum, and installs to `~/.local/bin/runyte` without sudo. Run the
same command again to update. Add `~/.local/bin` to your `PATH` if needed.
Linux requires glibc 2.35 or newer; Alpine/musl is unsupported. See the
[installation guide](docs/user-guide.md#install-and-update-with-curl) for
requirements, script review, version selection, and custom install locations.

On Windows, download the x86-64 archive from
[GitHub Releases](https://github.com/runyte/runyte/releases), or install through cargo.

### Install through cargo

Installing from crates.io requires Rust 1.88 or newer and a C compiler:

```sh
cargo install runyte --locked
```

### Build from source

To build from a clone:

```sh
cargo build --release
./target/release/runyte README.md
```

### Post-install setup

Git features require `git` on `PATH`; language servers are installed separately.

The [shell directory guide](docs/user-guide.md#change-the-shell-directory-on-exit)
explains how `:quit-here` and both wrappers work.

#### Linux and macOS

On Linux and macOS, it is advised to set the following in your `~/.bashrc` or `~/.zshrc`:
```sh
# To run it using ru instead of runyte ;)
alias ru=runyte

# To use runyte as the default editor, e.g. in git, Claude Code, Codex
export EDITOR='runyte --wait'
export VISUAL='runyte --wait'

# To support :quit-here - quit to the directory selected in the file explorer
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

Linux clipboard integration uses the first available of `wl-clipboard`, `xclip`,
or `xsel`.

#### Windows

On Windows, in PowerShell, add `Set-Alias ru runyte` to your
[`$PROFILE`](https://learn.microsoft.com/powershell/module/microsoft.powershell.core/about/about_profiles)
and start a new PowerShell session. To use `:quit-here` on Windows,
save [runyte.ps1](contrib/runyte.ps1) in a stable local location and dot-source it from your PowerShell profile:
```powershell
. 'C:\Tools\Runyte\runyte.ps1'
```
The wrapper is tested with Windows PowerShell 5.1; PowerShell 7 has not been validated.

### First run

```sh
runyte
runyte .
runyte src/main.rs
runyte +120:8 src/app.rs
runyte -a
runyte -a /path/to/notes
```

Run `runyte --help` for the complete command-line interface.

## Help

Key sequences show hints. `:tutorial` provides an
interactive introduction, `Space ?` opens contextual help, and `:help` opens
the complete manual.

- [User guide and command reference](docs/user-guide.md) (long; an AI agent can search it for you)
- [Frequently asked questions](docs/faq.md)
- [Configuration](docs/user-guide.md#configuration) and [example](config.example.yaml)
- [Language-server setup](docs/lsp/README.md)
- [Third-party notices](THIRD_PARTY_NOTICES.md)

## Contributing

Contributions are welcome in the form of:

- Feature requests and bug reports through [GitHub Issues](https://github.com/runyte/runyte/issues).
- [Plugin development](docs/plugins/authoring.md), in any programming language.

See the [contributing guide](CONTRIBUTING.md) for how to describe a feature
request, report a bug, or share a plugin.

## The name

Runyte is pronounced *“roon-ite”* and blends rune, byte, Rust, and unite `¯\_(ツ)_/¯`.

## Acknowledgements

Runyte's selection-first model and much of its keymap language come from
[Helix](https://helix-editor.com/); highlighting builds on
[tree-house](https://github.com/helix-editor/tree-house) and
[Tree-sitter](https://tree-sitter.github.io/). The interface uses
[Ratatui](https://ratatui.rs/), [Crossterm](https://github.com/crossterm-rs/crossterm),
and [Ropey](https://github.com/cessen/ropey).

The explorer follows ideas from [Oil.nvim](https://github.com/stevearc/oil.nvim),
and jump labels from [hop.nvim](https://github.com/smoka7/hop.nvim). See
[third-party notices](THIRD_PARTY_NOTICES.md) for the full credits and licenses.

## License

Runyte is licensed under the [Mozilla Public License 2.0](LICENSE).
