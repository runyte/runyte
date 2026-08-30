# Runyte

[![CI](https://github.com/runyte/runyte/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/runyte/runyte/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-%E2%89%A583%25-brightgreen)](context/reference/test-coverage.md)

Runyte is a terminal workspace for software development, built around a modal
text editor. It brings together an editable file explorer, terminal
multiplexing, persistent detachable sessions, fuzzy search across files,
buffers, and terminals, and integrated Git workflows in one coherent interface.

Runyte aims to provide a consistent environment—with a unified theme and
keybindings—for people who regularly move between terminals and Git worktrees,
search and edit files, and copy text between editor buffers and CLI
applications. It is especially well suited to running multiple coding agents in
parallel, but it does not depend on them. If your current setup combines
separate tools or plugins for text editing, terminal multiplexing, file
management, and Git, Runyte may be for you.

At its core is a fast terminal editor written in Rust. Common motion
keybindings will feel familiar to users of both Vim and Helix. Its
multiple-selection model and selection-first editing are closer to Helix than
Vim, while some commands and workflows are distinctly Runyte's own.

Website: [runyte.com](https://runyte.com) ·
Documentation: [user guide](docs/user-guide.md)

## Features

The editor, files, terminals, Git, and language tools are not separate plugins
with separate interaction models: they share the same panes, commands,
workspace, and visual language.

| Area | Built in Runyte |
| --- | --- |
| **Modal editor** | Selection-first, multicursor, Vim and Helix bindings |
| **File management** | Editable directory explorer |
| **Terminal sessions** | Multiplexing, scrollback |
| **Git** | Status, diffs, staging, commits, pull, push, branches, worktrees, blame, stashes |
| **Language** | Tree-sitter for 26 languages, asynchronous LSP |
| **Sessions** | Optional client–server mode, detachable clients |

Across all of them: the same keybindings, one shared clipboard, fuzzy search
over anything, jump anywhere, and switching between Git worktrees.

See the [full user guide](docs/user-guide.md) for behavior, limits, and the
complete command reference.

### Standalone and persistent modes

A **workspace** is one project directory and its editor scope. In standalone
mode, the default, that state belongs to the TUI process. In persistent mode a
local host retains the workspace while the TUI attaches and detaches, much like
a multiplexer session:

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

### Opinionated by default

Runyte has a YAML configuration file, but its built-in features are intended to
work together without requiring a personal configuration or a plugin stack.
The tradeoff is deliberate: the feature set is opinionated, while the
keybindings, themes, buffer types, and safety rules can be designed as one
whole. There is no plugin system today and no implementation roadmap has been
approved.

## Screenshots

![Runyte displaying the About screen, Rust source code, Git status, and an integrated terminal in a tiled workspace.](https://runyte.com/images/screenshots/runyte_1.webp)

*Editor, Git status, and an integrated terminal in a persistent workspace.*

![Runyte displaying Rust source code beside the file explorer and its contextual help manual.](https://runyte.com/images/screenshots/runyte_2.webp)

*Source editing alongside the file explorer and contextual help.*

![Runyte displaying terminal review, the About screen, and the file explorer in three panes.](https://runyte.com/images/screenshots/runyte_3.webp)

*Terminal review, the welcome screen, and the file explorer in a three-pane
layout.*

![Runyte displaying the persistent-session picker with session and Git worktree details.](https://runyte.com/images/screenshots/runyte_4.webp)

*The persistent-session picker with session details and worktree metadata.*

![Runyte displaying the command palette filtered to Git commands, above a terminal pane, an open Markdown file, and Rust source.](https://runyte.com/images/screenshots/runyte_5.webp)

*The searchable command palette, narrowed to the Git namespace.*

Full-size versions are on the
[screenshots page](https://runyte.com/screenshots/).

## Performance

The benchmark opens generated documents at 500, 5,000, and 50,000 lines. Each
size is written twice with byte-identical content: the `.txt` file measures
reading and drawing without language processing, while the `.lua` file makes
Neovim, Helix, and Runyte parse the same document with a single Tree-sitter Lua
grammar.

Startup time is in milliseconds as **first output / settled frame**: the first
byte written to the terminal, then the moment drawing goes quiet. Each result
is the median of 10 runs in a 120×40 pseudo-terminal with an empty editor
configuration.

| Fixture | LOC | Size | Neovim | Helix | Runyte |
| --- | ---: | ---: | ---: | ---: | ---: |
| `short.txt` | 0.5k | 17 kB | 6 / 18 | 17 / 18 | **5 / 6** |
| `medium.txt` | 5k | 171 kB | 6 / 17 | 19 / 20 | **6 / 7** |
| `long.txt` | 50k | 1.7 MB | 6 / 22 | 22 / 23 | **16 / 17** |
| `short.lua` | 0.5k | 17 kB | 6 / 30 | 22 / 23 | **10 / 12** |
| `medium.lua` | 5k | 171 kB | 6 / 46 | 48 / 50 | **28 / 29** |
| `long.lua` | 50k | 1.7 MB | 6 / 175 | 214 / 215 | **150 / 152** |

Absolute values are machine-specific. See the
[benchmark methodology](benchmarks/README.md) for how each fixture is measured
and [startup performance](context/reference/startup-performance.md) for the
machine, versions, and idle cost behind this result set.

## Installation

Runyte currently supports **Linux and macOS**. Windows support is planned for a
future release, but current releases should not be considered
Windows-supported. Building requires Rust 1.88 or newer and a C compiler for
the bundled Tree-sitter grammars.

```sh
cargo install runyte --locked
runyte README.md
```

To build from a clone:

```sh
./build.sh --release
./target/release/runyte README.md
```

Some useful starting points:

```sh
runyte                    # open the built-in introduction
runyte .                  # edit the current directory as an explorer
runyte src/main.rs        # edit a file
runyte +120:8 src/app.rs  # open at a one-based line and column
runyte --persistent       # attach to or start this workspace's session
runyte -a /path/to/notes  # initialize if needed, then attach persistently
```

Run `runyte --help` for the complete command-line interface.

### Change the shell directory on exit

Like Yazi, Runyte cannot change the working directory of the shell process
that launched it. The `:quit-here` command can hand its chosen directory back
through this Bash/Zsh wrapper:

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

After reloading the shell configuration, `:quit-here` (or `:qh`) exits with
normal unsaved-change protection and moves the shell to the active explorer
directory or active file's parent. Ordinary `:quit` leaves the shell directory
unchanged. See
[Shell-directory handoff in the user guide](docs/user-guide.md#change-the-shell-directory-on-exit)
for persistent-mode behavior and the `:quit-here!` force variant.

## Help

Runyte is designed to teach its interface while you use it. Starting any
command family—`Space`, `g`, `z`, `m`, `Ctrl-w`, or a view-specific
namespace—opens a hint popup with the available continuations, their
descriptions, and any reason a command is currently unavailable. The `:` palette
is searchable and uses the same command registry. After a command completes, the
interaction line shows the exact keys or command that were entered together with
its description or result, for example `g l (Move to line end)`; failures and
unavailable actions are explained there too.

Key execution, help, hints, and command descriptions all come from the same
registry, so the documentation shown inside the editor cannot silently drift
from what the keys do.

- `:tutorial` opens a guided two-pane introduction to modes, selection-first
  editing, search, multiple carets, command namespaces, panes, buffer types,
  the explorer, terminal sessions, jump history, and persistent sessions.
- <kbd>Space</kbd> <kbd>?</kbd> opens contextual help for the current buffer
  type.
- `:help` opens the general manual, and `:help <topic>` jumps to a topic such
  as `git`, `search`, `mouse`, or `lsp`.

### Documentation

- [User guide and complete feature reference](docs/user-guide.md)
- [Keybindings and command reference](docs/user-guide.md#key-bindings)
- [Configuration](docs/user-guide.md#configuration)
- [Diagnostics and logging](docs/user-guide.md#diagnostics-and-logging)
- [Language-server setup and examples](docs/lsp/README.md)
- [Runyte and Helix keymap differences](context/reference/helix-keymap-v1.md)
- [Example configuration](config.example.yaml)
- [Third-party notices](THIRD_PARTY_NOTICES.md)

### Diagnostics and logging

Runyte keeps a small local log of warnings, errors, and — when asked — more
detailed lifecycle events, so a failure can still be read after the process is
gone. The process that owns editor state owns its log: beneath the configured
workspace state directory, normally `.runyte/`, a standalone editor writes
`standalone-<pid>.log` and a persistent host writes `host.log`. `:log-open`
opens whichever belongs to the process holding the workspace, and
`:service-health` names its owner, level, and resolved path.

```sh
runyte -v                 # info; -vv adds debug and -vvv adds trace
runyte --log /tmp/run.log # write somewhere else
runyte --session-restart -vv   # a running host keeps its logger until restarted
```

Records never contain document text, selections, clipboard or terminal
contents, environment values, or language-server message bodies. They do contain
local paths and process metadata, so review a log before sharing it. The
[diagnostics guide](docs/user-guide.md#diagnostics-and-logging) documents
rotation, retention, and error behavior.

## The name

**Runyte** is pronounced *“roon-ite.”* The name combines **rune**, a small mark
that carries human meaning; **byte**, a small unit of machine-readable
information; and **unite**, because Runyte brings several terminal tools into
one interface.

## Acknowledgements

Runyte's selection-first editing model and much of its keymap language come
from [Helix](https://helix-editor.com/), though common motions also have
Vim-style aliases; [the keymap register](context/reference/helix-keymap-v1.md)
records where Runyte and Helix deliberately differ. Syntax highlighting is
built on [tree-house](https://github.com/helix-editor/tree-house), the Helix
project's tree-sitter highlighter and bindings, under the MPL-2.0.

The language layer rests on [Tree-sitter](https://tree-sitter.github.io/) and
the grammar and query authors listed in
[third-party notices](THIRD_PARTY_NOTICES.md), which also credits the seven
upstream palettes behind Runyte's built-in themes. The interface is drawn with
[Ratatui](https://ratatui.rs/) over
[Crossterm](https://github.com/crossterm-rs/crossterm), and every buffer is a
[Ropey](https://github.com/cessen/ropey) rope.

The editable directory explorer follows the design
[Oil.nvim](https://github.com/stevearc/oil.nvim) established, and the `g w`
jump labels take their visual and narrowing model from
[hop.nvim](https://github.com/smoka7/hop.nvim).

See [third-party notices](THIRD_PARTY_NOTICES.md) for the full list of
acknowledgements and the licenses they carry.

## License

Runyte is licensed under the [Mozilla Public License 2.0](LICENSE).
