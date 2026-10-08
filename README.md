# Runyte

[![CI](https://github.com/runyte/runyte/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/runyte/runyte/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/badge/coverage-%E2%89%A589%25-brightgreen)](context/reference/test-coverage.md)

https://github.com/user-attachments/assets/86edfda0-f8cb-4999-b35d-4f450f4ad3c3

*[Watch the 40-second demo](https://runyte.com/videos/runyte-demo.mp4?v=acb069b9306f): modal editing, search, file navigation, terminals, Markdown, persistent sessions, and a four-pane workspace. Matrix → gruvbox.*

Runyte is a fast modal text editor for focused work. Everything is accessed
through keybindings. No need to remember them though — every keybinding sequence
shows hints with possible completions.

Runyte can do **quite a lot**:
- multipane text editing with aligned indentation and language/file overrides
- terminal multiplexing
- file browsing and management
- Git, including reviewed merges, conflict resolution, branch/worktree
  management, and a cached commit network
- fuzzy finding across all files, terminals, and buffers
- plugins written in any language
- LSP support (code analysis, formatting, jumping between functions and variables)
- Tree-sitter support for 31 languages
- three modes: `ide` for a project, `ide+mux` to attach and detach while terminals keep running and unsaved buffers wait, and `editor` (`runed`) for quick edits anywhere
- word completion based on text from all open buffers
- smart file path completion (searching from current buffer and from project dir)

It also has features that make it **pleasant to work with AI agents** — managing
several at once and keeping up with their output:
- Git worktree support and fast switching
- built-in [MCP server](docs/mcp.md) for sharing buffer and terminal contents with agents: `runyte mcp --identity <name>`
- `Ctrl-g` in Claude Code or Codex attaches to the parent Runyte instance, instead of running a new one
  (after [setting `EDITOR`](#post-install-setup))
- Markdown formatting with `?`, including wide tables
- Image pasting with `Ctrl-v` or `Alt-v`
- Jump to any link with `gf` — works with file paths, Markdown links, web links (opens in a browser)

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
Use `Space d t` to toggle a narrow directory tree at the left of the editor,
or `Space d d` to reveal the active file and focus the tree. Enter opens a
selected file directly, or asks for a numbered destination when several panes
are open. Expanded directories update automatically as files change outside
Runyte. `n`, `d`, `m`, and `r` create, delete, move, and rename entries; only
delete asks for confirmation. Use `gg`/`ge`/`G` to jump to the first/last row,
`.` to toggle dotfiles, `/` to search visible entry names, and `gw` to label
onscreen entries and jump to one. While the tree
is focused, `:q`, `:wc`, and `:close` hide it. `Tab` toggles the bottom legend. Drag the right
border to resize it, or set `editor.directory_tree_width` in `Space o o`.

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

| Mode | Start with | What you get |
| --- | --- | --- |
| `ide` (default) | `runyte` | A workspace: Git, language servers, MCP, plugins, and terminals. Quitting stops it. |
| `ide+mux` | `runyte --mux`, or `runyte -a` | The same workspace kept alive by a local **host** while a **client** provides the terminal interface, so you can detach, reattach, and switch between projects or Git worktrees. |
| `editor` | `runyte --editor`, or `runed` | Editing files and directories anywhere, with no workspace, Git, language servers, MCP, plugins, or terminals. |

`ide` and `ide+mux` need a workspace: a Git repository, or a directory you
initialized with `runyte --init DIRECTORY`. Outside one they say so instead of
starting.

Start Runyte in mux mode with:

```sh
runyte -a  # attach to a session or start a new one
```

There are multiple ways to create a new session when you are already in Runyte:
- `Space Space` to open the session manager
- open the file explorer (`Space e`), navigate to another dir with `-` (go up) and `Enter` (go into) and press `Tab s` to start a new session in that dir
- open the integrated terminal, navigate to another dir and type `runyte -a` again

Switch between sessions with `Space Space`, or with `Shift-Left` and `Shift-Right`.

## Quick edits with runed

`runed` is Runyte in editor mode, whatever your configuration says. It edits a
file or browses a directory anywhere, writes nothing beside it, and the Finder
searches the directory you are in. The installer puts `runed` next to
`runyte`; after `cargo install`, add it yourself with
`ln -s runyte ~/.cargo/bin/runed`.
Runyte recognizes when it is launched as `runed`, so the symbolic link
(e.g. `ln -s /path/to/runyte runed`) alone is enough to select editor mode,
just like `runyte --editor`.

Use it wherever a program asks for an editor, and with `sudoedit` for system
files:

```sh
export EDITOR=runed
export SUDO_EDITOR=runed
sudoedit /etc/fstab
```

## Plugins

Much of what Neovim users assemble from plugins comes built into Runyte:
language servers, Tree-sitter highlighting, Git integration, fuzzy finding,
a file manager, terminals, and completion. Plugins therefore have a different
job. They don't change how the editor behaves; they connect separate programs
that integrate closely with it, adding their own commands, panes, prompts, and
documents. A database browser, an SFTP client, or a task tracker, for example.

That makes Runyte plugins narrower than Neovim's. They currently can't draw
highlights or inline text in your buffers, react to what you type, provide
completions or language-server features, or run built-in commands.

Plugins can be written in **any programming language**. Each one is a program
you enable, and it talks to Runyte in JSON over stdin/stdout.

Official Runyte plugins are in development:

- [**ru-time**](https://github.com/runyte/ru-time) — a task list and time tracker with task notes.
- [**ru-dbviewer**](https://github.com/runyte/ru-dbviewer) — browse SQLite and PostgreSQL databases and run SQL from editor buffers.

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

### Experimental native window (this branch)

The native window is built from this checkout with GPUI, Zed's Rust UI
framework. Install [Rust through rustup](https://rustup.rs/) and the platform
dependencies below before building. Use a current stable Rust toolchain for
the optional GPUI dependency graph.

Native builds embed JetBrainsMono Nerd Font (Medium, Medium Italic, Bold, and
Bold Italic); no system font installation is needed. The font is redistributed
under SIL OFL 1.1 with its [licenses and attribution](licenses/jetbrains-mono/README.md).
Terminal builds continue to use the font selected by your terminal emulator.

**macOS**

Install [Xcode](https://apps.apple.com/us/app/xcode/id497799835), launch it once,
and install its macOS components. GPUI compiles Metal shaders, so the full
Xcode installation is required in addition to the command-line tools. With
[Homebrew](https://brew.sh/) installed:

```sh
xcode-select --install  # If the command-line tools are not installed yet.
sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
sudo xcodebuild -license
brew install cmake pkgconf poppler
```

See the [upstream macOS build prerequisites](https://zed.dev/docs/development/macos)
if Xcode is installed in a different location. Poppler is available through
[Homebrew's formula](https://formulae.brew.sh/formula/poppler).

**Debian / Ubuntu**

```sh
sudo apt update
sudo apt install build-essential pkg-config cmake clang \
  libfontconfig1-dev libfreetype6-dev libxcb1-dev libx11-xcb-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libssl-dev \
  libvulkan1 mesa-vulkan-drivers poppler-utils
```

**Fedora**

```sh
sudo dnf install gcc gcc-c++ pkgconf-pkg-config cmake clang \
  fontconfig-devel freetype-devel libxcb-devel libX11-devel \
  libxkbcommon-devel libxkbcommon-x11-devel wayland-devel openssl-devel \
  vulkan-loader mesa-vulkan-drivers poppler-utils
```

**Arch Linux**

```sh
sudo pacman -Syu --needed base-devel pkgconf cmake clang fontconfig freetype2 \
  libxcb libx11 libxkbcommon libxkbcommon-x11 wayland openssl \
  vulkan-icd-loader poppler
```

Linux also needs a working Vulkan driver for the GPU. Debian/Ubuntu and Fedora
commands above include Mesa's drivers; NVIDIA installations should use their
distribution's matching NVIDIA driver. On Arch, install `vulkan-radeon` for AMD
or `vulkan-intel` for Intel, for example `sudo pacman -S --needed vulkan-radeon`.
For other distributions, consult the
[upstream Linux dependency instructions](https://zed.dev/docs/development/linux).

Poppler supplies `pdfinfo`, `pdftoppm`, and `pdftotext` for PDF rendering and
text selection. It is a runtime dependency only for PDFs; image viewing needs
no external decoder. If the build dependencies are already installed, only
install the PDF tools: `brew install poppler` on macOS,
`sudo apt install poppler-utils` on Debian/Ubuntu,
`sudo dnf install poppler-utils` on Fedora, or
`sudo pacman -S --needed poppler` on Arch. Fedora packages these tools in
[`poppler-utils`](https://packages.fedoraproject.org/pkgs/poppler/poppler-utils/).
Verify that all three are available in the same shell used to launch Runyte:

```sh
command -v pdfinfo pdftoppm pdftotext
```

Then build and open a window:

```sh
cargo run --features native -- --window
cargo run --features native -- --window --editor /path/to/file
```

For a desktop launcher and the Runyte icon on Linux (required by Wayland):

```sh
python3 contrib/native/package.py linux --binary target/debug/runyte
```

Register again if you move the binary or switch to `target/release/runyte`.
On macOS, use the [native desktop packaging guide](contrib/native/README.md)
to create a local `Runyte.app` with the same icon and bundled font notices.

The window keeps Runyte's cell layout, themes, command palette, key hints,
configured bindings, splits, and integrated terminals. It adds no toolbars.
Open an image or PDF with `:open`, the explorer, the directory tree, or a
startup filename to view it inside a pane. On PDFs, `j`/`k` select adjacent
pages and `42gg` jumps to page 42. Escape opens the PDF's page buffer: move or
search to a page and press Enter to display it. Another Escape returns to the
source directory; images return there directly. Escape first dismisses an
open overlay or clears a media selection. `Space e` opens the source directory
directly, with the file selected.

`+`/`-` zoom, `z f` fits, and `z h/j/k/l` pans. Ctrl-wheel zooms at the pointer;
middle-drag pans. Select PDF text with left-drag or image regions with
Shift-drag, then `y` copies the selection.

The experiment targets Linux and macOS; Windows and persistent-session window
attachments are not implemented. macOS installation prerequisites are documented
above, but the native frontend still needs hands-on macOS validation.

See [native-window details and limits](docs/user-guide.md#experimental-native-window).
The normal terminal executable remains available without `--window`.

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
