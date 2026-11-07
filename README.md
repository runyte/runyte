# Runyte

[![CI](https://github.com/runyte/runyte/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/runyte/runyte/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-%E2%89%A589%25-brightgreen)](context/reference/test-coverage.md)

https://github.com/user-attachments/assets/5f039873-8232-4ad9-b089-0d54b2b05e85

*[Watch the 45-second demo](https://runyte.com/videos/runyte-demo.mp4?v=7b63a9df8e582e64): editing, terminals, Markdown, persistent sessions, and the desktop window with animated images, PDFs, and pane dragging.*

Runyte is a Helix-style modal text editor living in your terminal or its own native window.
It supports multiple panes, and each pane shows a text file, a terminal, or a file manager.

Three *modes* are available:
- `editor` - text editor + file manager
- `ide` - text editor + file manager + LSP + terminals
- `ide+mux` - text editor + file manager + LSP + terminals + multiple persistent sessions

In the `ide+mux` mode Runyte works like a terminal multiplexer. You can close it (detach)
and open it again (attach) and you're back where you left off - no work is lost and
all terminals and AI agents keep doing what they were. The `ide+mux` mode replaces the need to use tools like tmux.

There are also two Runyte *editions*:
- terminal edition - run Runyte in your terminal of choice
- desktop edition - run Runyte in its own native window

The desktop edition replaces the need to use separate terminal emulators -
Runyte owns it all. In addition, the desktop edition can display images and PDFs
out of the box.

In other words, you can choose how you want to use Runyte.
It can be just a text editor or a full replacement of many terminal apps
that traditionally needed to be carefully wired together to feel like a
consistent terminal environment.

This project is well tested. Test coverage is about 92% of lines, and CI requires at least 89%.

Runyte supports all major platforms, but it is used by the devs mainly on Linux and macOS.
The desktop edition runs on Linux and macOS only.

| CPU    | System     | Automated tests | Prebuilt release | Hands-on use |
|--------|------------|:---------------:|:----------------:|:------------:|
| x86-64 | Linux      | ✅ | ✅ | ✅ |
| ARM64  | Linux      | ❌ | ✅ | ❌ |
| x86-64 | macOS      | ❌ | ✅ | ❌ |
| ARM64  | macOS      | ✅ | ✅ | ✅ |
| x86-64 | Windows 11 | ✅ | ✅ | 🟡 |
| ARM64  | Windows 11 | ❌ | ❌ | ❌ |

🟡 Occasional hands-on testing.

If you are using Runyte on other platforms - let us know!

Website: [runyte.com](https://runyte.com) ·
Documentation: [user guide](docs/user-guide/README.md) ·
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

Use `Space w ...` or `Ctrl-w ...` commands to manage panes. Move the active
pane to the left/bottom/top/right edge of the layout with `Ctrl-w H/J/K/L`.
With the mouse, drag shared borders to resize panes, Alt-left-drag inside a
pane to rearrange it, or Shift-Alt-left-drag to expand it into neighbouring
panes. See [pane management](docs/user-guide/panes.md#pane-keys) for details.

Use `Space d t` to toggle a narrow directory tree at the left of the editor,
or `Space d d` to reveal the active file and focus the tree. `n`, `d`, `m`, and
`r` create, delete, move, and rename entries. `Tab` toggles a key legend. See
the [directory tree](docs/user-guide/files-and-buffers.md#directory-tree-sidebar)
for details.

## Terminals

Type `:terminal` (or `:t`), or press `Space t n` or `Ctrl-w t`, to start a
terminal in the current pane. A new terminal starts in Insert mode, where keys
go to the running program. Press `Ctrl-\` to switch to Normal mode: the output
keeps running, but keys go to Runyte. Press `Ctrl-\` again to review the
output. This freezes a snapshot of the output, which you can move around,
search with `s` or `/`, and copy with `y`. Press `i` to go back to Insert mode.

## Modes

Runyte runs in one of three modes, chosen by a flag or by the `mode` setting.
The status line names the one you are in.

| Mode | Start with |
| --- | --- |
| `ide` (default) | `runyte` |
| `ide+mux` | `runyte --mux`, or `runyte -a` |
| `editor` | `runyte --editor`, or `runed` |

`ide` and `ide+mux` need a workspace: a Git, Jujutsu, or Subversion working
copy, or a directory holding a `.runyte/` state directory. `ide` creates
`.runyte/` in the launch directory when there is none; `ide+mux` needs one to
exist already. `runyte --init DIRECTORY` makes any directory a workspace. See
[starting a workspace](docs/user-guide/workspaces-and-persistent-sessions.md#starting-a-workspace)
for the exceptions, such as your home directory.

To start a new session from inside Runyte:
- `Space Space` opens the session manager
- in the file explorer (`Space e`), go to another directory and press `Tab s`
- in an integrated terminal, `cd` to another directory and run `runyte -a`

Switch between sessions with `Space Space`, or with `Shift-Left` and `Shift-Right`.

## Quick edits with runed

`runed` is Runyte in editor mode, whatever your configuration says. It edits a
file or browses a directory anywhere, writes nothing beside it, and the Finder
searches the directory you are in. The installer puts `runed` next to
`runyte`. After `cargo install`, create it as a symbolic link:
`ln -s runyte ~/.cargo/bin/runed`.

Use `runed` for standalone edits and with `sudoedit` for system files. To edit
inside an existing persistent session, use the [`--wait` setup](#post-install-setup).

```sh
export SUDO_EDITOR=runed
sudoedit /etc/fstab
```

## Installation

Choose an edition above. [INSTALLATION.md](INSTALLATION.md) covers dependencies,
platform-specific instructions, source builds, shell setup and updates.

### Easy install

Install or update the **terminal edition** on Linux or macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/runyte/runyte/main/install.sh | sh
```

The script verifies the release checksum and installs `runyte` and `runed` in
`~/.local/bin`, without sudo. Add that directory to `PATH` if needed. Linux
archives require glibc 2.35 or newer. Add `sh -s -- --desktop` in place of `sh`
to install the [desktop edition](INSTALLATION.md#install-script-desktop-edition)
instead. For Windows, use the x86-64 ZIP from
[GitHub Releases](https://github.com/runyte/runyte/releases) and follow the
[Windows instructions](INSTALLATION.md#windows).

### Install through cargo

Install the **terminal edition** with Rust 1.88 or newer and a C compiler:

```sh
cargo install runyte --locked
```

### Desktop edition

Install it with `install.sh --desktop` as shown above, or download the Linux
**desktop** archive for your CPU from
[GitHub Releases](https://github.com/runyte/runyte/releases) and follow the
[desktop installation steps](INSTALLATION.md#desktop-edition). Linux desktop
archives require glibc 2.39 or newer and a working Vulkan driver. On macOS 11 or
later, download the signed and notarized `Runyte-X.Y.Z.dmg` from releases and
drag Runyte into Applications.

The desktop executable also runs in a terminal:

```sh
runyte --window --mux  # window with a persistent session
runyte --mux           # terminal frontend, same persistent sessions
```

### Build from source

After installing the [build dependencies](INSTALLATION.md#build-from-source),
choose the executable to build:

```sh
cargo build --release --locked                    # target/release/runyte
cargo build --release --locked -p runyte-desktop  # target/release/runyte-desktop
```

Building does not install either executable. The [source installation
steps](INSTALLATION.md#install-a-source-build) explain how to install the desktop
build as `runyte`.

### Post-install setup

For Git and external-editor requests from Codex or Claude Code, set both
variables in `~/.bashrc` or `~/.zshrc`:

```sh
export EDITOR='runyte --wait'
export VISUAL='runyte --wait'
```

Start `runyte --mux` (or `runyte --window --mux`) in your project, then run the
agent in an integrated terminal. Its `Ctrl-g` edit opens in the parent
persistent session; `:wbc` saves and closes the buffer so the agent resumes.
Every client and host must use the same Runyte version. See [shell setup and
source-build paths](INSTALLATION.md#shell-and-external-editor-setup) for details,
Windows settings and optional `:quit-here` integration.

### First run

```sh
runyte --editor notes.md  # edit anywhere, without a workspace
runyte --mux              # open a persistent session in a project
```

Use `runyte --help` for all command-line options.

## Help

Key sequences show hints. `:tutorial` provides an
interactive introduction, `Space ?` opens contextual help, and `:help` opens
the complete manual.

- [User guide and command reference](docs/user-guide/README.md)
- [Frequently asked questions](docs/faq.md)
- [Configuration](docs/user-guide/configuration.md#configuration) and [example](config.example.yaml)
- [Language-server setup](docs/lsp/README.md)

## Contributing

Bug reports, feature requests, pull requests, and
[plugins](docs/plugins/authoring.md) are welcome. Small, focused pull requests
can be submitted directly. For a substantial feature or a change to editing
behavior, open an [issue](https://github.com/runyte/runyte/issues) first so we
can discuss the approach. See the [contributing guide](CONTRIBUTING.md) for
details.

## The name

Runyte is pronounced *“roon-ite”* and blends rune, byte, Rust, and unite 🤷.

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
