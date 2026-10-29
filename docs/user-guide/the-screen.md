## The screen

### Screen layout

```text
 session strip (optional, persistent sessions only)
┌ pane title ───────────────┐┌ pane title ───────────────┐
│ gutter │ buffer viewport  ││ gutter │ buffer viewport  │   editor area
│        │                  ││        │                  │
└───────────────────────────┘└───────────────────────────┘
 global status line
 interaction line
```

| Name | What it is |
| --- | --- |
| **Runyte screen** | The complete terminal surface. |
| **Editor area** | The upper part, holding one or more panes. |
| **Pane** | A **pane border**, a **pane title** in its top border, and a **pane body** inside it. |
| **Gutter** | Part of the pane body: line numbers, soft-wrap markers, syntax-fold markers, and Git change marks. |
| **Content padding** | Optional space in the pane body, used by aligned generated pages. |
| **Buffer viewport** | Where buffer text is shown and, for editable buffers, changed. |
| **Global status line** | Mode, workspace directory, active-buffer state, cursor, Git/LSP state, and unread notifications. |
| **Interaction line** | An active prompt or the last action echo. |

These names are also the vocabulary of help and source documentation, and any
future extension surface must inherit them. The canonical definitions live in
[`context/reference/ui-vocabulary.md`](../../context/reference/ui-vocabulary.md).

**Which surface a task gets** depends on its lifetime and interaction, not on
the shape of its box:

| Surface | Used for |
| --- | --- |
| **Buffer** (or a pane-backed filterable list) | A navigable result. Keeps movement, selection, search, splits, help, and buffer management. |
| **Special buffer** | Contents Runyte assembles rather than reads as file text: the explorer, config, Git views, notifications, help, the about page. |
| **Picker overlay** | A transient choose-one request. |
| **Context overlay** | Assistance tied to a source position. |
| **Confirmation overlay** | A pending prepared operation. |
| **Interaction line** | Short scalar input. |
| **Input overlay** | Richer validated input. |

Special buffers:

- remain full buffers while displayed, may be shared by panes, and may be
  editable or read-only;
- are retained while clean, up to the eight most recently active. Activating a
  ninth retires the least recent one once it is detached;
- stay available while dirty, until saved or discarded.

A rendered Markdown page is the exception: it is neither counted nor retired
while its document is open, and it closes with that document.

A pathless scratch buffer is ordinary text, not a special buffer.

For example, `Space g l` opens a Git-log buffer for ordinary browsing, while
`Space g f` opens a fuzzy commit picker that disappears after one choice. Both
open the same retained special commit-detail buffer when a commit is selected.

**Pane titles** name the buffer kind:

- ordinary paths are prefixed `[file]`;
- directory paths are prefixed `[explorer]`;
- virtual views keep their bracketed names, such as `[git status]` and
  `[git branches]`;
- `[+]` marks unsaved changes and `[RO]` a read-only buffer.

### Global status line

```
 NOR │ ide │ Workspace: /home/me/code/runyte [+]   412:17 · 34% │ 3 sel │ main ~1 │ rust-analyzer 0E 2W
```

| Part | Meaning |
| --- | --- |
| `NOR` | The mode, in the current mode's caret colour (see [Mode carets](themes.md#mode-carets)). The rest of the row keeps the theme's ordinary background. |
| `ide` | The mode: `ide` keeps live state in the TUI process, `ide+mux` keeps it in a persistent session, and `editor` has no workspace at all. See [Workspaces and persistent sessions](workspaces-and-persistent-sessions.md#workspaces-and-persistent-sessions). |
| `Workspace: …` | The current workspace directory. When it does not fit, the beginning is replaced with `...` and the identifying end kept. [Editor mode](workspaces-and-persistent-sessions.md#editor-mode) labels it `Directory: …` and shows no Git information. |
| `[+]` / `[STALE]` / `[RO]` | The active buffer has unsaved changes / the path it shows (a file or an explorer's directory) disagrees with the accepted baseline / is read-only. |
| `412:17 · 34%` | Cursor line and column, and how far through the buffer it is. |
| `↪ 412:17 · 34%` | The same position with `editor.soft_wrap` enabled. The arrow matches the soft-wrap continuation marker in the gutter. |
| `3 sel` | Selection count, shown above one. |
| `main ~1` | Git branch and outstanding changes. See [Status line and gutter](git.md#status-line-and-gutter). |
| `rust-analyzer 0E 2W` | Language-server summary. |
| `E1 W2 I3` | Unread notification counts, in their semantic theme colours. At narrow widths they compact to the highest severity plus the total, such as `E4`. |

Fields after the position appear only when they apply. Pane titles carry file
and buffer identity. The active theme is not shown; `Space o t` shows and
changes it.

**The percentage:**

- The first line reads `0%` and the last `100%`. No line between them does: a
  cursor one rounding step from either end reads `1%` or `99%`.
- A one-line buffer reads `100%`.
- It follows the cursor, not the topmost visible line, so `Z j` and the other
  view-scroll keys leave it alone until the cursor moves.

### Interaction line

The interaction line teaches each command as you use it:

| Result | Echo |
| --- | --- |
| No more specific result | The typed keys and the command description: `g l (Move to line end)` |
| A useful result | The result: `Space e (opened /path/to/dir)`, `:c (closed file)` |
| Failed | `binding (action · failed: message)` |
| Unavailable | `binding (action · unavailable: message)` |
| Unbound keys | `No binding: g z` |

- Commands that wait for a character keep showing the actual keys typed,
  including configured bindings and counts, and say what to type next.
  `m r` asks for the existing pair; `m r (` asks for its replacement.
  Find, replace, surround, register, and named-macro commands show this guidance
  until their last operand arrives. `Esc` cancels the pending input.
  Two-key jump labels also show the first label key while waiting for the
  second; completion, cancellation, and invalid input leave visible feedback.
- An active prompt uses the line temporarily. Errors, warnings, service output,
  and other notifications never replace the prompt or the echo.
- A failure carries the outcome's own message, not only the fact that it
  failed.
- A message longer than the space left, or spanning several lines, is cut to
  fit and ends with `...`. `:not` keeps the complete text. A prompt being typed
  is never cut, since its cursor position depends on the full text.
- A mistyped sequence (`No binding: g z`) is also shown in the key hints, but
  is not kept as a notification: it says nothing worth reading later, and a
  burst of mistyping would otherwise be the largest and only unbounded source
  of unread notifications.

**Pasting into prompts.** Single-line prompts and filters reject a whole paste
that contains control characters, including a trailing newline or tab.

- The text, cursor, and selected result stay unchanged, and the prompt shows
  `Control characters are not allowed; nothing was inserted` until the next
  input.
- This applies to the command line, search and rename prompts, Finder and list
  queries, session-directory queries, and typed Git confirmations.
- Literal backslash sequences such as `\n` are ordinary prompt text.
- Buffers and terminal sessions still accept multiline paste. Terminal paste
  actions, including macOS `Cmd-v`, keep line breaks even when the terminal
  sends them as carriage returns, and keep existing LF and CRLF separators.

### Notifications

`:notifications` (alias `:not`) opens `[notifications]`: one searchable,
read-only buffer with the retained history, newest first.

Each entry has:

- a local `YYYY-MM-DD HH:MM:SS` timestamp,
- an `ERROR`, `WARNING`, or `INFO` severity assigned by Runyte,
- a source, a title, and retained multiline details.

**Reading and unread counts:**

- Opening the buffer acknowledges the entries retained at that moment.
- New notifications arrive without moving focus and are unread.
- Consecutive identical notifications merge: their timestamp and occurrence
  count update and they become unread again.

**Severity** describes the underlying condition, not whether the action
completed:

| Severity | Examples |
| --- | --- |
| `INFO` | Expected context refusals, unavailable actions, empty search results |
| `WARNING` | Refusals that protect data or consistency, such as saving a read-only buffer or a file that changed on disk |
| `ERROR` | A failure in Runyte or an external dependency: a failed Git command, host operation, file write, or language-server exchange |

So the interaction line can say an action `failed` while its notification is
`INFO`.

**Bounds:**

- The configured entry count (`notifications.history_limit`), plus at most
  1 MiB per notification and 8 MiB per workspace.
- Git failures keep labelled stdout and stderr within the 1 MiB budget, enough
  for thousands of ordinary lines, and add a visible truncation marker when a
  hostile hook exceeds it. The rest of an overlong stream is still drained
  while the subprocess runs, so keeping less text never breaks an otherwise
  successful hook or helper.
- Successful asynchronous Git output updates the action echo while it is still
  current. Multiline output, or output whose echo has been superseded, is kept
  as `INFO`. The `:lsp-status` report updates its echo the same way, and is
  always kept as `INFO` as well.

### Help, hints, and the tutorial

| Command | What it opens |
| --- | --- |
| `Space ?` | Contextual help for the current buffer type |
| `:help` or `:?` | The general Runyte manual |
| `:help <topic>` | The manual at a section, such as `:help regex`, `search`, `mouse`, `git`, `sessions`, or `lsp` |
| `:about` | Logo, version, and a short getting-started guide |
| `:tutorial` | A guided, hands-on introduction |
| `:grammar` | The active editing grammar |

#### Key hints

Press a prefix such as `g`, `Space`, or `Ctrl-w` and a popup lists what can
follow.

- The default grammar groups commands under labelled `Space` namespaces. The
  popup shows one namespace row at a time, then the namespace's entries when
  you enter it.
- Short keys stay as fast or compatibility bindings. A row naming two keys,
  such as `Space s a, &`, runs the same command either way, so the namespace
  teaches the short spelling rather than hiding it.
- Bindings added by plugins or configuration under an unlabelled prefix fold
  the same way: several bindings under `Space =` appear as one `Space =` row,
  named after their shared command name or by how many there are.
- `Space l` (**Language (LSP)**), `Space x` (**Syntax (Tree-sitter)**), and
  `Space g` (**Git**) are dimmed with a reason when the service is not ready:
  no language server, no parser, or no Git / no repository. They stay
  navigable so each command can explain itself; LSP status and restart can
  stay usable without an attached document.

In the popup:

- `Ctrl-n` and `Ctrl-p` scroll without affecting the pending keys. In Normal
  and Select, Up and Down scroll too, unless that arrow completes a binding.
- Scroll controls appear in the title when the entries do not fit.
- Up to three columns are used when complete rows fit. Ide-mode and attached
  persistent clients use the same layout.

#### Contextual help (`Space ?`)

- One document per buffer type, not per mode. Normal and Select bind the same
  keys, so a text buffer has a single `TEXT` document describing both.
- In a plugin view it lists the actions Tab offers there. A plugin that
  supplies help topics also explains the view's workflow, such as browsing a
  table's rows or inspecting one record.
- Key lists are read from the keymap when help opens, so they follow your
  configured bindings. In a read-only view, keys that would only report a
  refusal are left out.

The generated key lists cover each mode:

- **Normal and Select:** buffer-specific keys and `Tab` actions first, then
  prefixes whose hint popup teaches the rest, then direct keys grouped as
  letters and punctuation, `Ctrl` chords, `Alt` chords, and named keys.
  Shifted `<` and `>` also carry searchable `Shift-<` and `Shift->` spellings.
- **Insert and Replace:** shared keys once, then separate lists for keys that
  differ. A read-only view says when these modes are unavailable. Terminal
  Insert lists keys sent to the child and Runyte-owned exceptions separately.
- **Command:** prompt controls under their own heading. The command prompt
  handles these itself, outside the keymap.

#### Help buffers

`:help guide` (also `:help reference`) jumps to the complete user guide,
assembled from the same chapters as the website. The other help topics remain
short introductions with keys adapted to the active keymap.

- Both kinds of help are ordinary read-only buffers: they scroll, search,
  split, and close with `q`, `:c`, or `Space b c`. Nothing is truncated to fit
  the window, and opening one kind does not replace the other.
- The about page and help colour section titles, commands, key bindings, file
  paths, web links, and technical examples with the theme's syntax colours.
  They are not Markdown: visible backticks stay ordinary searchable, copyable
  text, and colouring changes no buffer coordinate and makes nothing
  clickable.
- A read-only buffer is marked `[RO]` in the pane title and the status line,
  and its help says so: `Help · RUNYTE · GIT STATUS · Read-only`. The status
  line describes the buffer on screen; the help title describes the buffer
  type it is about, so help for an editable type is not titled read-only.

#### The about page

`:about` is centred against the pane, not against fixed columns in its text.
Resize the window or split the view and the page moves, without a character
being rewritten. The space around it is drawn, not stored, so search,
scrolling, and clicks land on the text. A page taller than its pane starts at
its first row and scrolls like any read-only buffer.

#### The tutorial

`:tutorial` opens two ordinary panes: read-only instructions and disposable
scratch text. Complete each exercise by performing the described actions.
The instructions announce success while the result stays visible. When ready,
type `:tutorial next` and press Enter; unfinished exercises cannot be skipped.
In Terminal Insert mode, press `Ctrl-\` before entering the command.

- **First choice:** show Vim-like motion spellings, Helix-like ones, or both.
  This changes only the spellings in the tutorial, never Runyte's behavior or
  keymap.
- **Lessons:** modes; selection-first editing with characterwise and
  `x`/`X` whole-line selections; search; multiple carets; `Space` discovery;
  `Ctrl-w` pane commands.
- **Views:** scratch, generated, and editable explorer buffers versus terminal
  pane content. You open the explorer, return with `Alt-o`, create a terminal,
  and close that terminal session through its manager.
- **Final lessons:** `Ctrl-o`/`Ctrl-i` jump history, the ide/mux
  boundary, and pointers to `:help` and `Space ?`.

| Command | Effect |
| --- | --- |
| `:tutorial` | Resume the live lesson |
| `:tutorial next` | Continue after completing the current exercise |
| `:tutorial reset` | Start over |
| `:tutorial sessions` | Open the persistent-session lesson directly |

#### Editing grammar

`:grammar` reports the active Runyte grammar. `helix` is accepted as a
configuration and command alias for `runyte`.

### Mouse

Mouse support needs a terminal with mouse reporting and `editor.mouse: true`
(the default).

| Action | Effect |
| --- | --- |
| Click | Focus a pane and place the caret |
| Shift-click | Extend the selection |
| Ctrl-left-click in the native window (Cmd-left-click on macOS) | Follow the file path or web link under the pointer, like `gf`, including live terminal output |
| Drag | Select, entering Select mode |
| Drag past the top or bottom row | Scroll and keep extending |
| Wheel | Scroll the pane under the pointer |
| Drag a shared border | Resize the adjacent aligned panes; unequal spans resize their shared split |
| Alt-left-drag inside a pane | Rearrange panes using the [pane drop outline](panes.md#pane-keys) |
| Shift-Alt-left-drag inside a pane | Extend the pane into neighbouring panes |
| Right-click a selection | Copy all selections to the system clipboard, like `Space c y` |
| Double-click a directory tree row | Toggle the directory or open the file, like Enter |

- **Autoscroll** follows visual rows, including soft wrapping and collapsed
  folds. Move back inside the pane or release the button to stop.
- **Right-click copy** leaves the selections alone and reports
  `right mouse click (yanked to system clipboard)`.
  The native window uses the same behavior for document and terminal-review
  selections; right-drag in PDF and image previews pans instead.
- **Character rule.** The pointer names the character under it, not the gap
  before it, so a drag covers the characters it started and ended on, in
  either direction. Pressing past a line's end places the caret on its last
  character, or past it in Insert mode, as keyboard motion does.
- **Native link clicks.** Ctrl-left-click (Cmd-left-click on macOS) uses the
  clicked character, ignoring existing selections. It follows the same paths,
  Markdown labels and heading links, and web URLs as `gf` in document views and
  terminal panes. Live terminals stay in input mode; reviewed terminals use
  their frozen text. Supported wrapped terminal URLs resolve from any of their
  rows. This gesture, including its drag and release, is never sent to the child.
  Extra modifiers retain their ordinary mouse behavior; prompts and overlays
  keep ownership of input. The interaction line names the gesture and its
  result, such as
  `Ctrl-left-click (opened https://example.com in the default browser)`.
- **Terminal panes.** A click focuses a live terminal in Insert mode; a
  reviewed terminal stays in Normal/review until a terminal insert key returns
  to the live screen. A left drag in review selects cells and enters Select
  mode, and right-click copies that selection. Clicking another pane focuses
  it in Normal mode. Dragging from a document press still selects and enters
  Select mode.
- Pointer coordinates go through the same fold- and wrap-aware rows used for
  drawing, so collapsed lines and wide Unicode glyphs never disagree.
- Passive pointer motion never clears key hints or status and does not
  redraw.
- Mouse capture is released on every exit and error path.

**Native text selection.** Capturing the mouse takes over the terminal's own
text selection. To keep that instead, set `editor.mouse: false` and restart.

### Jump history

| Key | Moves through |
| --- | --- |
| `Ctrl-o` / `Ctrl-i` | Every recorded position, including terminal surfaces and positions within one file |
| `Alt-o` / `Alt-i` | The same history, stopping only on a different buffer or terminal surface |

`Alt-o` leaves a document you have read through in one press, rather than one
per section.

Closing a buffer with `:c` or `Space b c` keeps every pane. Each pane returns
to its own most recently used live buffer, or gets a new scratch buffer when
none remains.

### Contextual actions (`Tab`)

In Normal and Select modes, `Tab` asks what can be done with the thing under
the cursor.

- **Git views** open an action menu. Each row has four aligned columns: the
  mnemonic, one word naming the action, whether it acts on the row or the whole
  buffer, and a sentence explaining it.
- **Language-server buffers** request code actions and open the code-action
  picker, or report that none are available.
- **Other views** (explorer, lists, plugin views) offer their own actions.

In an action menu:

| Key | Effect |
| --- | --- |
| Arrows or `j` / `k` | Move |
| Enter | Run the selected action |
| The mnemonic | Run that action directly |
| Escape | Cancel |

Mnemonics belong only to the open menu, so `s`, `n`, `p`, and the rest keep
their normal meanings in every buffer.

<!-- navigation -->
[Contents](README.md) · [Previous: Install and run](install-and-run.md) · [Next: Key bindings](key-bindings.md)
