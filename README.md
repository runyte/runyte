# Runyte

[![CI](https://github.com/runyte/runyte/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/runyte/runyte/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-%E2%89%A589%25-brightgreen)](context/reference/test-coverage.md)

![Runyte 0.2.0 with a clickable strip of running persistent sessions above the editor.](https://runyte.com/images/screenshots/session-strip.webp?v=5f3ba82c64b1)

*Switch projects. Keep buffers and terminals running. · ocean-dark*

**Runyte** is a terminal workspace built around a modal text editor.

Optional persistent mode lets you **detach and return later**. A local host keeps
your terminal processes and language servers running.

The **fuzzy Finder** searches your project, including files, unsaved buffers,
and terminals. Search by name or by content.

Use **consistent keys** to move between files, buffers, terminals, and Git
worktrees.

Run **Claude Code**, **Codex** or any other CLI agent in a terminal pane.
Share the clipboard with the editor. With [Runyte set as their editor](#editing-agent-prompts), `Ctrl+G`
opens your prompt in the same persistent workspace. Save it and return to
the agent.

Press `?` to read Markdown as a **formatted page**, including tables.

**Paste images** with `Ctrl+V`. Runyte saves them in the project's temporary cache
and inserts a Markdown link into your document.

Press `gf` on a file path in your text to open it. Images and other binary
files open in an external program you choose.

Project goals:

- Maximum performance and rock-solid stability.
- Minimal UI. Maximum focus.
- Coherent keybindings with constant feedback. Start a command sequence to
  see the next keys.
- One consistent theme for editing and terminals.

Runyte currently supports Linux and macOS.

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
| **Language** | 26 bundled Tree-sitter grammars and asynchronous LSP |
| **Sessions** | Standalone or persistent workspaces, session switching, and `$EDITOR`-compatible `--wait` |
| **Interface** | Registry-backed key hints and help, themes, settings, and notifications |

Language servers require permission per workspace; editing and Tree-sitter
features remain available without them. Use `:lsp-trust` to change permission.

Runyte uses optional YAML configuration. Explicitly enabled
[experimental process plugins](docs/plugins.md) can register commands and transform
multiple selections in one undoable edit. The opt-in
[application API](docs/plugins/applications.md) adds native application views,
a local file manager, background jobs, explicit text operations and provider-backed
UTF-8 editing with conditional remote saves and remote conflict comparison while the broader application platform
remains in development.
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
client to another host.

```sh
runyte --persistent
runyte --session-list
runyte --session-list --include-hidden  # include isolated live sessions
runyte --persistent api   # attach to a session by ID, name, or directory
```

Persistent mode keeps open and unsaved buffers, selections, registers, syntax
state, diagnostics, Git projections, language-server processes, and live
terminal sessions for the lifetime of the host process. It is local, supports
one interactive TUI at a time, and is currently Unix-only. It does not claim
survival across a host crash, force-stop, logout, reboot, or machine failure.

The [workspace and persistent-session guide](docs/user-guide.md#workspaces-and-modes)
documents attachment, switching, lifecycle commands, and `--wait`.

### Editing agent prompts

`Ctrl+G` opens the external prompt editor in
[Codex CLI](https://learn.chatgpt.com/docs/cli-customization#prompt-editor) and
[Claude Code](https://code.claude.com/docs/en/interactive-mode#general-controls).
When the agent runs in a Runyte persistent terminal with `EDITOR` and `VISUAL`
set to `runyte --wait`, that request connects to the existing host and opens
the prompt as a buffer in the same pane:

```text
Codex / Claude Code in a Runyte terminal
    |
    +-- Ctrl+G --> runyte --wait --> Existing host
                                       |
                                       +-- Prompt buffer in the same pane
                                               |
                                               +-- :wq --> Back to the agent
                                                           with the edited prompt
```

Set `export EDITOR='runyte --wait' VISUAL='runyte --wait'` in the integrated
shell before launching the agent. The attached client displays the prompt;
`:wq` saves it, completes the request, and restores the agent's terminal.
See the [navigation guide](docs/user-guide.md#session-and-destination-navigation)
for editor environment handling and the full return-to-terminal behavior.

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
runyte --persistent
runyte -a /path/to/notes
```

Run `runyte --help` for the complete command-line interface. To let
`:quit-here` change the launching shell's directory, use the
[Bash/Zsh wrapper](docs/user-guide.md#change-the-shell-directory-on-exit).

## Persistent sessions

Persistent mode keeps buffers, selections, Git state, language servers, and
terminal sessions alive while the TUI is detached. Each workspace has a local
host and accepts one interactive TUI at a time. Persistent mode is Unix-only
and does not survive host termination or reboot.

```sh
runyte --persistent
runyte -a WORKSPACE
runyte --session-list
```

Inside the editor, the session strip and keyboard shortcuts switch running
sessions. Commands in an integrated terminal can use `runyte -a` to switch the
outer TUI or `runyte --wait` for editor requests. See the
[persistent-session guide](docs/user-guide.md#workspaces-and-modes).

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

## Status

Runyte is pre-1.0. Current development focuses on reliability, performance,
and Linux/macOS support; Windows support is planned. Report bugs through
[GitHub Issues](https://github.com/runyte/runyte/issues).

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
