# Runyte

[![CI](https://github.com/runyte/runyte/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/runyte/runyte/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-%E2%89%A589%25-brightgreen)](context/reference/test-coverage.md)

Runyte is a fast modal terminal editor that combines selection-first editing,
file management, terminal multiplexing, Git workflows, and language tools in
one interface.

The editor, explorer, terminals, Git views, and language tools share the same
panes, theme, command registry, and clipboard. Runyte's keymap is inspired by
Helix's selection-first model and includes familiar Vim motions; it deliberately
differs from both.

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

Runyte uses optional YAML configuration and does not currently support plugins.
See the [user guide](docs/user-guide.md) for complete behavior and limits.

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

![A process monitor, Git branches, and the file explorer arranged in one Runyte workspace.](https://runyte.com/images/screenshots/terminal-git-explorer.webp)

![Runyte editing Rust with multiple selections beside a terminal and file explorer in a light theme.](https://runyte.com/images/screenshots/light-theme.webp)

![Runyte workspace search matching across files and terminal output with a live preview.](https://runyte.com/images/screenshots/workspace-search.webp)

![Runyte's persistent-session picker showing workspace activity and details.](https://runyte.com/images/screenshots/sessions.webp)

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
