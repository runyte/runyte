# Runyte

Runyte is a fast, feature-rich modal terminal editor written in Rust. It brings
together workflows that usually live in four separate tools:

- a modal text editor such as Neovim, Helix, or Emacs;
- a terminal file manager such as nnn or Yazi;
- a terminal multiplexer such as tmux or Zellij; and
- a Git interface such as Lazygit.

Runyte does not aim to beat each specialized tool at everything it does. It is
for people who would rather edit files, reshape directories, run interactive
programs, and work with Git inside one coherent interface—with one keybinding
language, one clipboard model, and one theme.

Its editing model is selection-first and inspired by Helix: move to select,
then act. The keymap deliberately differs where Runyte has chosen another
workflow, especially for search and macros. Common motions also have Vim-style
aliases.

## What Runyte brings together

The table is a map of each tool's primary, built-in experience, not a quality
ranking. **●** means a central or built-in workflow, **◐** means a narrower
feature or one commonly supplied by an extension, and **—** means that the area
is outside the tool's main scope. The specialized tools are often deeper in
their own domains; Runyte's distinction is that all four workflows are designed
as one editor.

| Tool | Text editing | File management | Terminal sessions | Git workflows | LSP and syntax | How it grows |
| --- | --- | --- | --- | --- | --- | --- |
| **Runyte** | **● selection-first modal editor** | **● editable directory explorer and project finder** | **● workspace-scoped terminal multiplexer with detach/reattach** | **● status, diffs, staging, history, blame, branches, worktrees, and stashes** | **● built-in LSP and Tree-sitter** | Opinionated built-ins and YAML configuration; no plugins yet |
| [Neovim](https://neovim.io/) | ● modal editor | ◐ usually plugins | ◐ integrated terminal buffers | ◐ usually plugins | ● built-in LSP and Tree-sitter APIs | Lua and plugins |
| [Helix](https://helix-editor.com/) | ● selection-first modal editor | ◐ picker and explorer workflows | — | ◐ change indicators | ● built-in LSP and Tree-sitter | Configuration; no stable plugin system |
| [Emacs](https://www.gnu.org/software/emacs/) | ● programmable editor | ● built-in file manager (Dired) | ◐ shells and persistent server clients | ◐ commonly Magit | ● built-in language tooling | Emacs Lisp and packages |
| [nnn](https://github.com/jarun/nnn) | — | ● terminal file manager | ◐ shell and tool integration | — | — | Plugins and shell integration |
| [Yazi](https://yazi-rs.github.io/) | — | ● terminal file manager | ◐ shell and tool integration | — | — | Lua plugins and configuration |
| [tmux](https://github.com/tmux/tmux) | — | — | ● terminal multiplexer and sessions | — | — | Configuration, scripts, and plugins |
| [Zellij](https://zellij.dev/) | — | — | ● terminal multiplexer and sessions | — | — | Layouts and plugins |
| [Lazygit](https://github.com/jesseduffield/lazygit) | — | ◐ repository file views | — | ● focused Git interface | — | Configuration and custom commands |

### Where the specialized tools go further

Runyte trades specialization and extensibility for integration. Neovim and
Emacs have much larger extension ecosystems; Helix offers a more focused
selection-first editing experience; nnn and Yazi go deeper into dedicated file
management; tmux and Zellij provide more mature general-purpose multiplexing;
and Lazygit supports Git workflows beyond Runyte's intentionally bounded
interface. If one of those domains dominates a workflow, its specialist will
often be the stronger tool.

Runyte is younger, has no plugin system, currently supports only Linux and
macOS, and limits persistent mode to one local interactive client. Its appeal
is the coherence of the combined workflow, not feature parity with every
specialist.

## Discoverable by design

Runyte is designed to teach its interface while you use it. Starting any
command family—`Space`, `g`, `z`, `m`, `Ctrl-w`, or a view-specific
namespace—opens a hint popup with the available continuations, their
descriptions, and any reason a command is currently unavailable. This applies
to application commands, editor motions, window commands, Git actions,
terminal actions, and special-buffer actions rather than only to a single
which-key menu.

The `:` palette is searchable and uses the same command registry. After a
command completes, the interaction line shows the exact keys or command that
were entered together with its description or result, for example
`g l (Move to line end)`. Failures and unavailable actions are explained there
too, while the searchable notification center retains errors and warnings that
need a closer look.

Key execution, help, hints, and command descriptions all come from the same
registry, so the documentation shown inside the editor cannot silently drift
from what the keys do.

## Opinionated by default

Runyte has a YAML configuration file, but its built-in features are intended to
work together without requiring a personal configuration or a plugin stack.
The tradeoff is deliberate: the feature set is opinionated, while the
keybindings, themes, buffer types, and safety rules can be designed as one
whole.

There is no plugin system today and no implementation roadmap has been
approved.

## Highlights

- Selection-first modal editing with multiple selections, counts, named
  registers, macros, transactional undo, jumplists, structural text objects,
  syntax-aware indentation, folds, and Unicode-aware wrapping.
- Statically linked Tree-sitter highlighting for 18 languages, with no grammar
  download or plugin manager required.
- Asynchronous language-server support for diagnostics, completion, hover,
  signature help, navigation, references, symbols, rename, code actions, and
  formatting. Word and path completion also work without a language server.
- An Oil-style editable directory explorer. Rename, move, copy, create, and
  delete entries with normal editor commands, then review an explicit
  filesystem plan before anything changes on disk. Deletions go to the trash
  unless permanent deletion is separately requested.
- Native fuzzy search across project files, file contents, open buffers, and
  terminals, plus a two-character `goto-word` jump over visible text.
- Nested splits, shared buffers, side-by-side live buffer comparisons, Zen and
  full-screen maximized panes, built-in light and dark themes, mouse support,
  and operating-system clipboard integration.
- Unix integrated terminal panes for shells, full-screen TUIs, development
  tools, and other interactive programs. Terminal review mode supports
  scrolling, search, selection, and copy while the child keeps running.
- Built-in asynchronous Git status, gutter marks, changed-file views, staging
  by file/hunk/supported line selection, commit, pull/push, branches,
  worktrees, history, blame, stashes, and aligned side-by-side diffs.
- A searchable notification center, contextual help for every buffer type,
  registry-backed settings and theme browsers, and completed-command feedback
  on the interaction line.
- Standalone operation by default, plus optional persistent mode that keeps
  unsaved buffers, editor state, language servers, and terminal children alive
  while the TUI is detached.

See the [full user guide](docs/user-guide.md) for behavior, limits, and the
complete command reference.

## Standalone and persistent modes

A **workspace** is one project directory and its editor scope. In standalone
mode, the default, that state belongs to the TUI process. In persistent mode a
local host retains the workspace while the TUI attaches and detaches, much like
a multiplexer session:

```sh
runyte --persistent
runyte --session-list
runyte --persistent api   # attach to a session by ID, name, or directory
```

Persistent mode keeps open and unsaved buffers, selections, registers, syntax
state, diagnostics, Git projections, language-server processes, and live
terminal sessions for the lifetime of the host process. It is local, supports
one interactive TUI at a time, and is currently Unix-only. It does not claim
survival across a host crash, force-stop, logout, reboot, or machine failure.

The [workspace and persistent-session guide](docs/user-guide.md#workspaces-and-modes)
documents attachment, switching, lifecycle commands, and `--wait`.

## Platform support

Runyte currently supports Linux and macOS. Windows support is planned for a
future release, but current releases should not be considered Windows-supported.

## Install and run

Runyte requires Rust 1.88 or newer and a C compiler for the bundled Tree-sitter
grammars.

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
```

Inside the editor, press <kbd>Space</kbd> then <kbd>?</kbd> for contextual
help. Press a prefix such as <kbd>Space</kbd>, <kbd>g</kbd>, or
<kbd>Ctrl-w</kbd> to see its commands immediately. Run `runyte --help` for the
complete command-line interface.

## Change the shell directory on exit

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

## Documentation

- [User guide and complete feature reference](docs/user-guide.md)
- [Keybindings and command reference](docs/user-guide.md#key-bindings)
- [Configuration](docs/user-guide.md#configuration)
- [Runyte and Helix keymap differences](context/reference/helix-keymap-v1.md)
- [Example configuration](config.example.yaml)
- [Third-party notices](THIRD_PARTY_NOTICES.md)

## The name

**Runyte** is pronounced *“roon-ite.”* The name combines **rune**, a small mark
that carries human meaning; **byte**, a small unit of machine-readable
information; and **unite**, because Runyte brings several terminal tools into
one interface.

## License

Runyte is licensed under the [Mozilla Public License 2.0](LICENSE).
