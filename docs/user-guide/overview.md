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
- Unicode-aware buffer positions and terminal widths: an emoji such as `🤷‍♀️`
  or `👍🏽`, or a letter with its accents, is one character to the caret,
  selections, deletion, wrapping and the integrated terminal

**Languages**

- Tree-sitter syntax highlighting for Python, Rust, Swift, C, C++, JavaScript,
  TypeScript, TSX, HTML, CSS, Go, Bash, Java, Kotlin, SQL, Lua, C#, Zig, CMake,
  Protobuf, Make, INI, Markdown, TOML, YAML, JSON, JSONC, Dockerfile, XML,
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
- Optional `--mux` local workspace persistence for unsaved editor,
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

The [key bindings](key-bindings.md#key-bindings) section records what each binding does here.
`context/reference/helix-keymap-v1.md` is the register of which bindings match
Helix and which do not.

<!-- navigation -->
[Contents](README.md) · [Next: Install and run](install-and-run.md)
