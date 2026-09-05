# Runyte

[![CI](https://github.com/runyte/runyte/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/runyte/runyte/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-%E2%89%A589%25-brightgreen)](context/reference/test-coverage.md)

What Runyte is depends on how you use it. It can be any or all of the following:

- a **fast modal text editor** with batteries included and no configuration
  required
- a multi-pane **file manager** that opens binary files in external applications
- a **terminal multiplexer** with fuzzy search across files, buffers, and
  terminals
- a **Git interface** with first-class worktree support

Runyte brings these tools together in a consistent terminal environment, with
one theme and one set of keybindings.

Editing is selection-first, with multiple selections inspired by Helix and
motions familiar to both Helix and Vim users.

Runyte’s immediate focus is performance, reliability, and a rock-solid
experience on macOS and Linux.

Website: [runyte.com](https://runyte.com) ·
Documentation: [user guide](docs/user-guide.md) ·
Changelog: [GitHub Releases](https://github.com/runyte/runyte/releases) ·
Community: [r/runyte](https://www.reddit.com/r/runyte/)

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

On first opening a workspace, Runyte asks whether language servers may run
project code. Editing and syntax highlighting work with LSP disabled;
`:lsp-trust` changes the workspace permission later.

Runyte can be configured through YAML, but its built-in features are designed
to work together without requiring a personal configuration or plugin stack.
It does not currently support plugins.

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

## Screenshots

![Runyte About screen showing the logo, version number, and getting-started keyboard shortcuts.](https://runyte.com/images/screenshots/about.webp)

*The About screen with Runyte's essential navigation and workspace shortcuts.*

![Runyte workspace with a process monitor and Git branch list on the left and a project file explorer on the right.](https://runyte.com/images/screenshots/terminal-git-explorer.webp)

*A process monitor, Git branches, and the file explorer arranged in one
workspace.*

![Runyte's workspace search matching htop across files and a terminal, with a result preview beside the match list.](https://runyte.com/images/screenshots/workspace-search.webp)

*Workspace-wide fuzzy search across files, buffers, and terminals, with a live
preview.*

![Runyte comparing the indexed and worktree versions of a README side by side, with additions and deletions highlighted.](https://runyte.com/images/screenshots/side-by-side-diff.webp)

*A side-by-side Git diff comparing the indexed file with the working-tree
version.*

![Runyte in a light theme with a process monitor and file explorer beside a Rust source file containing multiple selections.](https://runyte.com/images/screenshots/light-theme.webp)

*Terminal, file explorer, and multi-selection source editing in a unified light
theme.*

![Runyte showing Claude Code and OpenAI Codex in two terminal panes above a Git log and Git branch list.](https://runyte.com/images/screenshots/coding-agents.webp)

*Claude Code and OpenAI Codex working alongside the Git log and branch browser.*

![Runyte command palette filtered to Git commands over an open Rust source file.](https://runyte.com/images/screenshots/git-commands.webp)

*The searchable command palette filtered to Runyte's built-in Git commands.*

![Runyte session picker listing persistent workspaces with details for the selected runyte-dev session.](https://runyte.com/images/screenshots/sessions.webp)

*The persistent-session picker with activity, process, branch, directory, and
worktree details.*

Full-size versions are on the
[screenshots page](https://runyte.com/screenshots/).

## Performance

Runyte aims for top-notch performance in everyday editing. To check progress,
we benchmark startup, quitting, and idle cost against Neovim and Helix, and the
finder's fuzzy path matching against fzf. These measurements cover specific
tasks; they do not establish an overall fastest editor.

### Editor

The startup comparison measures readiness to edit: time from process launch
until one inserted space appears in the document. After timing ends, the
harness saves to a temporary path and verifies the whole file to confirm that
the edit was accepted. Each editor opens the same generated documents at 500,
5,000, and 50,000 lines, with no language assigned for `.txt` and Tree-sitter
Lua enabled for `.lua`.

Results below are median milliseconds from ten launches after a warm-up, with
isolated configuration and storage, measured on September 5, 2026 on an AMD
Ryzen AI 9 365 running Linux (Neovim 0.12.4, Helix 25.07.1, Runyte 0.1.10).

| Fixture | Lines | Size | Neovim (ms) | Helix (ms) | Runyte (ms) |
| --- | ---: | ---: | ---: | ---: | ---: |
| `short.txt` | 500 | 17 kB | 22.5 | 29.7 | 15.1 |
| `medium.txt` | 5,000 | 171 kB | 23.3 | 31.6 | 15.5 |
| `long.txt` | 50,000 | 1.7 MB | 25.3 | 31.1 | 23.0 |
| `short.lua` | 500 | 17 kB | 41.0 | 34.7 | 21.2 |
| `medium.lua` | 5,000 | 171 kB | 32.0 | 53.1 | 32.7 |
| `long.lua` | 50,000 | 1.7 MB | 32.2 | 295.7 | 157.5 |

The benchmark reports file loading and syntax completion separately: Neovim
can display an edit before its initial parse finishes, while Runyte prepares
syntax before showing document text. These timings cover one edit near the
start of each file. Small differences need to be read alongside the ranges
in the [detailed results](context/reference/startup-performance.md).

In the August 31, 2026 measurements (Runyte 0.1.6, Neovim 0.12.4, Helix
25.07.1), Runyte quit in 4–28 ms, Neovim in 2–6 ms, and Helix in
4–22 ms across these fixtures. With a Lua document open in a Git repository,
all three had a median idle CPU reading of 0.00% and no screen writes across
five ten-second windows; Runyte's CPU readings ranged from 0.00% to 0.10%.

### Finder

The finder's path scoring uses one thread for up to 2,048 candidates and
multiple threads for larger candidate sets when multiple cores are available.
This depends on how many candidates are being scored, not how many characters
you type. The final sort uses one thread. The benchmark's standalone Runyte
filter uses single-threaded scoring and sorting, so its timings do not measure
the complete interactive finder.

The table below compares complete command-line filters on the same 10,000
paths, including process startup, reading input, matching, sorting, and writing
results. Values are median milliseconds from 15 runs on September 4, 2026,
using Runyte 0.1.10 and fzf 0.74.2 on an AMD Ryzen AI 9 365 running Linux.
fzf's default matching uses multiple threads; the last column limits it with
`GOMAXPROCS=1`.

| Query | Runyte filter (one thread) | fzf (default threading) | fzf (one thread) |
| --- | ---: | ---: | ---: |
| Empty | 3.2 | 6.0 | 5.7 |
| `s` | 10.1 | 8.9 | 10.7 |
| `src` | 8.2 | 7.1 | 9.5 |
| `fpr` | 3.4 | 5.1 | 4.9 |
| `keymap` | 3.1 | 4.9 | 4.1 |
| `file_picker.rs` | 2.8 | 4.8 | 3.9 |
| `src/parser` | 3.4 | 5.0 | 4.9 |
| `parser test` | 4.0 | 6.2 | 11.7 |
| `zzqx` (no match) | 2.2 | 4.5 | 3.8 |

Neither leads on every query with default threading. Multi-term queries also
have different semantics: Runyte requires terms in the typed order; fzf does
not. Separately, the Runyte filter's in-memory ranking took 0.9–7.9 ms at
10,000 paths, rising to 49.3 ms for `s` at 100,000 paths. Those ranking-only
figures exclude input/output and cannot be compared directly with fzf's
whole-process timings. Neither measurement includes filesystem discovery,
previews, or drawing.

All timings are specific to the recorded machine and versions. Read more in
the [benchmark methodology](benchmarks/README.md), the
[startup, quit, and idle results](context/reference/startup-performance.md),
and the [fuzzy matching results](context/reference/fuzzy-matching.md).

## Installation

Runyte currently supports **Linux and macOS**. Windows support is planned for a
future release, but current releases should not be considered
Windows-supported.

Prebuilt archives are attached to each [GitHub Release](https://github.com/runyte/runyte/releases)
for these targets:

- `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`;
- `x86_64-apple-darwin` and `aarch64-apple-darwin`.

Download the archive matching the machine together with `SHA256SUMS`, compare
the archive's `sha256sum` (Linux) or `shasum -a 256` (macOS) output with its
line in that file, and extract it. Each archive contains a versioned directory
with the `runyte` executable, configuration example, and licence material.
The macOS executables are currently unsigned and are not notarized.

Installing from source requires Rust 1.88 or newer and a C compiler for the
bundled Tree-sitter grammars:

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

`Space f` (or `Space / f`) opens the Finder in name mode across files, open
buffers, and terminals; Tab switches to content mode over file lines,
authoritative buffer text, and decoded terminal output. Buffer search stays on
`s` for escaped literals and `/` for regular expressions, with `Space / s` and
`Space / /` widening the same two flavours to the whole workspace. Finder query
editing stays interactive while filesystem discovery, ranking, and previews
continue in the background. Retained workspace searches also traverse and
match in the background; accepting their prompt returns immediately, and the
result buffer opens when the identified request completes. While one is
pending, the status row keeps a rotating spinner directly beside
`Searching workspace`, the query, and its elapsed time.

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

## Roadmap

Feature-wise, Runyte is close to what I originally envisioned. Over the next
few months, development will focus on hardening the current implementation:
fixing bugs, expanding test coverage, and improving performance. The goal is
for Runyte to be fast and rock-solid on Linux and macOS. Windows support will
follow once that foundation is in place.

Want to help? Report bugs through [GitHub Issues](https://github.com/runyte/runyte/issues)
and join broader discussions on [r/runyte](https://www.reddit.com/r/runyte/).

## The name

**Runyte** is pronounced *“roon-ite.”* The name draws on several words:

- **rune**, a small mark that carries human meaning
- **byte**, a small unit of machine-readable information
- **Rust**, the language Runyte is written in
- **unite**, because Runyte brings several terminal tools into one interface

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
