# Runyte

[![CI](https://github.com/runyte/runyte/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/runyte/runyte/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-%E2%89%A589%25-brightgreen)](context/reference/test-coverage.md)

https://github.com/user-attachments/assets/cc77a90c-25e5-4b15-a1c9-f5da7f3f12fb

*[Watch the 60-second demo](https://runyte.com/videos/runyte-demo.mp4): from a Markdown prompt to Rust code, then find text across files and terminals.*

**Runyte** is a terminal workspace built around a modal text editor.

It supports **standalone** and **persistent** modes.
In standalone mode when you exit Runyte you quit everything it was running.
In persistent mode you can **detach and return later**, while a local host keeps
your terminal processes and language servers running, and unsaved buffers open.
The persistent mode is more powerful and preferred for focused work across
many projects.

Runyte integrates text editing, a file explorer, and terminal multiplexing under one
roof. It gives us some unique benefits:
- **Fuzzy Finder** searches your entire project, including files, unsaved buffers,
  and terminals
- **Consistent keys** to move across buffers, files, terminals, plugins
- With the optional [context bridge](bridges/runyte-context/README.md), agents can
  read and write to buffers and terminals, including sending messages to other agents 🤯
  (no worries - you still need to approve)
- Press `Ctrl+g` in **Claude Code** or **Codex** running in a Runyte terminal to edit
  your prompt in the same persistent Runyte session instead of starting a new one
  (it requires setting [Runyte as their editor](docs/user-guide.md#session-and-destination-navigation))

If you work with agents a lot, you'll also appreciate our **Markdown formatting**
with `?`, including wide tables and **image pasting** with
`Ctrl+V` (or `Alt-v` when a terminal reserves `Ctrl+V`). Runyte saves images
in the project's temporary cache and inserts a Markdown link into your document.

Press `gf` on a file path in a buffer or terminal review to open it. Markdown
link and image labels work too. Images and other binary files open in an external program you choose.
Web links (`https://`, `http://`, and `www.`) open in your default browser,
including links automatically wrapped across terminal review rows.

Project goals:

- Maximum performance and rock-solid stability.
- Minimal UI. Maximum focus.
- Coherent keybindings with constant feedback. Start a command sequence to
  see the next keys.
- One consistent theme for editing and terminals.

Linux and macOS provide the full feature set. Windows support in this source
tree includes standalone editing, `--wait`, file management,
integrated terminals, clipboard, language services, Git when installed, and the
optional context bridge, configured plugins and direct persistent attachment.
Native session controls and foreground hosts are available. See the
[Windows scope and requirements](docs/user-guide.md#windows-support).
The released 0.3.1 packages predate this Windows work.

Website: [runyte.com](https://runyte.com) ·
Documentation: [user guide](docs/user-guide.md) ·
Changelog: [GitHub Releases](https://github.com/runyte/runyte/releases) ·
Community: [r/runyte](https://www.reddit.com/r/runyte/)

## Features

| Area | Built in Runyte |
| --- | --- |
| **Editing** | Multiple selections, registers, macros, structural syntax tools, transactional undo |
| **Files** | Editable directory explorer with reviewed filesystem plans; unified file, buffer, and terminal search |
| **Terminals** | Interactive PTYs, scrollback, modal review, splits, persistent processes |
| **Git** | Status, diffs, staging, commits, pull, push, branches, worktrees, blame, stashes |
| **Language** | 31 bundled Tree-sitter grammars and asynchronous LSP |
| **Sessions** | Standalone or persistent workspaces, session switching, and `$EDITOR`-compatible `--wait` |
| **Interface** | Registry-backed key hints and help, themes, settings, and notifications |
| **Plugins** | Plugins written in any programming language, with native commands, views, and background work |

Language servers require permission per workspace; editing and Tree-sitter
features remain available without them. Use `:lsp-trust` to change permission.

See the [user guide](docs/user-guide.md) for complete behavior and limits.

### Workspaces, panes, and navigation

Everything belongs to a **workspace**: one project directory, its panes,
open buffers, and terminal sessions. Panes show what you are working on;
buffers hold editor content, and terminals run programs.

```text
Workspace
|
+-- Pane 1 ---- shows ----> Buffer A: src/main.rs
+-- Pane 2 ---- shows ----> Buffer A: same text, another view
+-- Pane 3 ---- shows ----> Terminal: shell, build, or coding agent
|
+-- Other open buffers and terminals, ready to switch to
```

A buffer can contain a file, scratch text, a directory listing, or a Git
view. Several panes can show the same buffer. Each terminal has at most
one visible pane, and hiding it leaves its program running.

| Where you want to go | Default keys |
| --- | --- |
| Another visible pane | `Ctrl-w h/j/k/l` |
| An open buffer or running terminal | `Space n` — Navigator |
| Any file, buffer, or terminal in this workspace | `Space f` — Finder; `Tab` switches between names and contents |
| Text in the current buffer | `s` for literal search; `/` for regex |
| Another persistent session | `Space Space` — session manager; `Shift-Left/Right` cycles running sessions |

`Ctrl-w n` also opens the Navigator while typing in a terminal. Finder searches
the current workspace; the session manager takes you between workspaces.

### Standalone and persistent modes

In standalone mode, the default, the workspace lives in one Runyte process.
In persistent mode, a local **host** keeps the workspace alive while a
**client** provides the terminal interface:

```text
Client (your Runyte screen)
    |
    +-- attach / detach --> Host
                             |
                             +-- Workspace
                                 panes, buffers, terminals
```

Each host serves one workspace. Switching persistent sessions connects the
client to another host. Persistent sessions survive detaching, but not a host
shutdown or reboot.

On Linux and macOS, add `-a` when starting Runyte to run in persistent mode:

```sh
runyte -a  # attach to a session or start a new one
```

Press `Space Space` to open the session manager in Runyte. On Windows, use
`runyte -a` for direct attachment; the manager can list, preview, rename and
stop native sessions, but cannot attach or switch between them.
On Unix, press `Shift Left` and `Shift Right` to quickly switch between sessions.

The [workspace and persistent-session guide](docs/user-guide.md#workspaces-and-modes)
documents attachment, switching, lifecycle commands, and `--wait`.

## Plugins

Official Runyte plugins are in development:

- [**ru-time**](https://github.com/runyte/ru-time) — a task list and time tracker with task notes.
- [**ru-dbviewer**](https://github.com/runyte/ru-dbviewer) — browse SQLite and PostgreSQL databases and run SQL from editor buffers.

Plugins can be written in **any programming language**. Each runs as an
explicitly enabled external process and exchanges bounded, newline-delimited
JSON with Runyte over stdin/stdout. The asynchronous host handles registered
commands, native views and input, background work, and capability grants.
Type `::` to browse plugin commands.
For example, ru-time offers `::time`, `::time-add`, and `::time-delete`.
Plugins can also provide configurable keybindings through the editor's regular help and hints.

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

GitHub Releases provide archives and checksums for x86-64 and ARM64 Linux and
macOS. The macOS executables are currently unsigned and not notarized.

Installing from crates.io requires Rust 1.88 or newer and a C compiler:

```sh
cargo install runyte --locked
```

Git features require `git` on `PATH`; language servers are installed separately.
Linux clipboard integration uses the first available of `wl-clipboard`, `xclip`,
or `xsel`.

To build from a clone:

```sh
./build.sh --release
./target/release/runyte README.md
```

Useful starting points:

```sh
runyte
runyte .
runyte src/main.rs
runyte +120:8 src/app.rs
runyte -a
runyte -a /path/to/notes
```

Run `runyte --help` for the complete command-line interface. To let
`:quit-here` change the launching shell's directory, use the
[shell wrappers](docs/user-guide.md#change-the-shell-directory-on-exit).

## Screenshots

![Runyte 0.2.0 displaying a Markdown document as a formatted page.](https://runyte.com/images/screenshots/rendered-markdown.webp?v=28c285c399fe)

*Read Markdown as a page. Return to source. · nordbones-dark-soft*

![Runyte 0.2.0 Navigator listing open buffers and running terminals.](https://runyte.com/images/screenshots/navigator.webp?v=fbd9ff1239c3)

*Jump between open buffers and terminals. · terafox-soft*

![Runyte 0.2.0 comparing indexed and working-tree Rust source with highlighted changes in rosebones-dark.](https://runyte.com/images/screenshots/side-by-side-diff.webp?v=e74adc8b6b70)

*Compare changes side by side. · rosebones-dark*

![Claude Code and OpenAI Codex in adjacent Runyte 0.2.0 terminal panes in frappe.](https://runyte.com/images/screenshots/coding-agents.webp?v=75cab6417c7e)

*Claude and Codex. Two terminals, one workspace. · frappe*

More examples are on the [screenshots page](https://runyte.com/screenshots/).

## Performance

Runyte benchmarks readiness to edit, quit and idle behavior, Finder ranking,
and persistent-session navigation. Reproducible harnesses and machine-specific
results live in the [benchmark guide](benchmarks/README.md),
[startup record](context/reference/startup-performance.md), and
[fuzzy-matching record](context/reference/fuzzy-matching.md).

## Help

Command prefixes open registry-backed key hints. `:tutorial` provides an
interactive introduction, `Space ?` opens contextual help, and `:help` opens
the complete manual. `:log-open` and `:service-health` help diagnose failures.

- [User guide and command reference](docs/user-guide.md)
- [Frequently asked questions](docs/faq.md)
- [Configuration](docs/user-guide.md#configuration) and [example](config.example.yaml)
- [Language-server setup](docs/lsp/README.md)
- [Runyte and Helix keymap differences](context/reference/helix-keymap-v1.md)
- [Diagnostics and logging](docs/user-guide.md#diagnostics-and-logging)
- [Third-party notices](THIRD_PARTY_NOTICES.md)

## Status and contributing

Contributions are welcome in the form of:

- Feature requests through [GitHub Issues](https://github.com/runyte/runyte/issues).
- Bug reports through [GitHub Issues](https://github.com/runyte/runyte/issues).
- [Plugin development](docs/plugins/authoring.md), in any programming language.

See the [contributing guide](CONTRIBUTING.md) for how to describe a feature
request, report a bug, or share a plugin.

## The name

Runyte is pronounced *“roon-ite”* and blends rune, byte, Rust, and unite.

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
