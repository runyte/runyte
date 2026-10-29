## Terminals

A terminal session runs a program — a shell, `htop`, `vim`, a coding agent —
inside a pane. It is pane content, not a buffer, and it outlives the pane
showing it.

### Starting a terminal

| Command | Runs |
| --- | --- |
| `:terminal` (`:term`, `:t`), `Space t n`, `Ctrl-w t` | `$SHELL` on Unix, `%COMSPEC%` (else `cmd.exe`) on Windows, in the editor working directory. Not available in [editor mode](workspaces-and-persistent-sessions.md#editor-mode). |
| `:terminal htop` | That command line |
| `:terminal-file-directory [command]` | From the active file's parent |
| `:terminal-directory-root [command]` | From the active explorer's root |
| `:terminal-selected-directory [command]` | From the selected directory entry |
| `:terminal-session-directory <number\|name>` | A shell in another terminal's last safe directory |
| `Tab t` in an explorer | A terminal in the directory the explorer shows |

- The pane's own buffer stays where it is; leaving the terminal shows it
  again.
- Unix splits command lines shell-style; Windows uses native
  double-quote/backslash syntax.
- **Directory reports.** Shells may report their directory with a bounded
  local OSC 7 `file:` URL. Remote hosts, control characters, non-file schemes,
  and paths that are not existing absolute directories are rejected.

#### Windows terminals

Windows terminals use ConPTY and need no Bash.

- Executables are found on absolute `PATH` entries using `PATHEXT`. The
  workspace is not searched implicitly.
- Quote paths with spaces:
  `:terminal "C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo`.
- Operators and batch files need an explicit shell:
  `:terminal cmd.exe /d /c "echo hello"`.
- For `cmd.exe /c` or `/k`, give exactly one command-string argument. Runyte
  adds `/s` and keeps that string as shell text, nested quotes included:
  - `:terminal cmd.exe /d /c "echo \"a b\""` prints `"a b"`;
  - `:terminal cmd.exe /d /c "\"C:\scripts\my task.cmd\""` runs a batch
    file whose path has spaces.
- PowerShell 7 and Git Bash are optional installations.
- Each terminal owns an independent process tree; closing it or quitting the
  editor ends that tree.
- Working directories need an ordinary Windows path shorter than 260 UTF-16
  units; paths needing extended-name semantics are refused before launch.
  `cmd.exe` also refuses UNC working directories, so use a shell that
  supports them. These limits do not affect which files the editor can open.

### Terminal modes

| Mode | Keys go to | Shown as |
| --- | --- | --- |
| Insert | The program, except Runyte's few exceptions | `[insert]` in the pane title |
| Live Normal | Runyte; the screen keeps updating | No title marker |
| Normal/review | Runyte; a frozen snapshot you can navigate and copy | Review marker in the title, grayed text |

```text
 INSERT ──Ctrl-\──▶ live NORMAL ──Ctrl-\ or a review command──▶ NORMAL/review
   ▲                    │                                           │
   └──────── i, a, o, or another terminal insert key ◀──────────────┘
```

The title answers whether typing reaches the child, so NORMAL is unmarked: the
mode line already says it.

#### Terminal Insert mode

Every key belongs to the program except:

| Key | Effect |
| --- | --- |
| `Ctrl-\` | Leave to live Normal mode |
| `Ctrl-w` | Start the window prefix |
| `Ctrl-h/j/k/l` | Move between panes, only with `editor.fast_pane_keys` on |
| `Shift-Left` / `Shift-Right` | Visit the previous / next running session, in mux mode (remappable) |

- `Escape`, `Ctrl-c`, `Ctrl-o`, `Space`, and ordinary keys reach the child
  unchanged.
- **Window keys from a terminal.** `Ctrl-w h/j/k/l` and their control-key
  aliases move to another pane at once. `Ctrl-w v/s` split, leaving the child
  live in the original pane; the new pane opens the working-directory
  explorer, as `Space E` does. `Ctrl-w f/z` toggle full-screen and zen from
  terminal Insert, live Normal, Normal/review, and Select without changing the
  mode. Bare `Ctrl-w`, or cancelling the prefix, keeps the current mode.
  Window actions never capture or discard a review snapshot.
- **Arriving at a pane.** Every focus route — these keys, pane cycling, a
  mouse click — enters a live terminal in Insert mode. A terminal with a
  captured review stays in review until `i`, `a`, `o`, or another insert key.
  A document reached from Terminal Insert starts in Normal.
- **`Ctrl-\` spelling.** On macOS, Runyte asks for unambiguous Ctrl-key reports
  without repeat and release events. A terminal without that protocol sends
  legacy control bytes, where `Ctrl-\` arrives as `Ctrl-4`; both work.

#### Review mode

**Review navigates and copies. It does not edit.** The screen is a picture of
the program's text, and the program owns its own input area, so an edit to
those cells would be an edit to a picture. Commands that would edit are
refused rather than applied to the buffer behind the pane.

A second `Ctrl-\`, or the first review command used from live Normal, captures
the bounded output as an immutable snapshot. New output keeps arriving behind
it.

- Motions move a visible review caret and keep it within the viewport margin.
- `v` enters Select mode; motions then extend inclusively through both the
  start and end characters, in either direction. `Escape` cancels a selection.
- The title marks review and newer output, and the reviewed text is grayed,
  so a frozen image never looks like a live program.
- Moving to another pane does not start review; returning to a reviewed
  terminal keeps its snapshot.
- A live terminal keeps its own colours. Alternate-screen review holds only
  the captured visible screen.

**Pasting back into the program:**

| Key | Sends | Afterwards |
| --- | --- | --- |
| `p` / `P` | Runyte's selected register | Insert mode, so the next key (usually Enter) reaches the child |
| `Space c p` / `Space c P` | The system clipboard | Stays in Normal mode |
| `u` | One delete per character of the last paste | Undoes that paste |

- Both routes leave review first and write at the child's real cursor, never
  into captured output.
- `u` works only while the paste is still the child's last input: any key
  typed into the terminal ends it. It is refused for a paste that ended a line
  the child has already run, since what ran is the child's. A shell at a
  prompt erases exactly the paste.

`i` leaves review and types again.

### Terminal keys

| Key | Action |
| --- | --- |
| `Space t n` or `Ctrl-w t` | Run `$SHELL` in this pane (`:terminal`, `:term`, `:t`; `:terminal <command>` runs something else) |
| `Space t t` | List the running and retained exited terminals and visit the chosen one (`:terminals`) |
| `Space t r` | Rename this pane's terminal (`:terminal-rename <name>`) |
| `Space t q` | Show this pane's buffer again, leaving the program running |
| `Space t y` | Copy this terminal's output into a read-only buffer (`:terminal-output`) |
| `Space t s` | Send the selection — or the whole buffer — to a terminal as one bracketed paste (`:terminal-send [number\|name]`) |
| `Tab`, then Close in `Space t t` | Explicitly end and forget the selected terminal |
| `Tab`, then Force kill in `Space t t` | Kill the selected terminal's process group and discard its output after confirmation; Enter confirms, Escape cancels |
| `Ctrl-w h/j/k/l` or `Ctrl-w Ctrl-h/j/k/l` in Terminal Insert | Move directly without capturing or discarding review; a live terminal destination starts Insert, a reviewed terminal stays in review, and a document destination starts Normal |
| `Ctrl-h/j/k/l` in Terminal Insert | The exact same destination behavior without the prefix, when `editor.fast_pane_keys` is on; the child stops receiving those four keys |
| `Ctrl-w w` in Terminal Insert | Cycle panes with the same live-terminal/reviewed-terminal/document destination behavior |
| `Ctrl-w v/s` in Terminal Insert | Create a vertical/horizontal split showing the working-directory explorer, while leaving the terminal child live without review |
| `Ctrl-\` in a terminal | First leave INSERT for live NORMAL, then enter review on the second press. Reported as `Ctrl-4` by terminals without the enhanced keyboard protocol, and both work |
| `i` / `a` in a terminal | Type again, returning to the live screen first |
| `h` / `j` / `k` / `l`, word, line, paragraph, and character-find motions | Move the terminal review caret; after `v`, extend an inclusive character selection in either direction |
| `%` in terminal review | Select all retained review text |
| `x` / `X` in terminal review | Select the current line, then grow the lower / upper edge on repeated presses without ever shrinking |
| `C` / `Alt-C` in terminal review | Add carets below / above at the same occupied terminal-cell column, skipping short rows |
| `Ctrl-u` / `Ctrl-d`, `Ctrl-b` / `Ctrl-f` | Move the review caret by half / full pages, keeping it visible |
| `gg` / `ge` in a terminal | Move to the oldest / newest rows in the captured review snapshot |
| `gf` in terminal review | Open the selected file path or web link, or the target under the caret; inferred paths at the end of a sentence ignore trailing punctuation if the literal name does not exist. Web links use the default browser, including links broken across rows by the terminal or by an agent's indented output |
| `gw` in terminal review | Label visible terminal words and jump to the chosen one |
| `f` / `F` / `t` / `T` in terminal review | Find characters in the snapshot |
| `s` / `/`, then `n` / `N` | Search an immutable terminal review snapshot by literal / regular expression and move among matches |
| `)` / `(` in terminal review | Move through search matches, like `n` / `N` |
| `y` / `Space c y` in terminal review | Copy the caret character or every selection, joined by newlines, to the unnamed register / system clipboard |
| `p` / `P` in a terminal | Leave review and send Runyte's selected register to the live program |
| `Space c p` / `Space c P` in a terminal | Leave review and send the system clipboard to the live program |
| `u` in terminal review | Take back the last paste, while it is still the child's last input |

Review search is case-insensitive for `s` and a regular expression for `/`,
with stable highlighted matches.

### Moving text in and out

**Out of a terminal.** `Space t y` (`:terminal-output`) copies the session into
an ordinary read-only buffer. That is real text: search, multiple selections,
`n`/`N`, and yank all work. `Ctrl-o` or `Alt-o` returns to the terminal, and
`Ctrl-i` or `Alt-i` goes back to the output.

**Into a terminal.** Write the text in an ordinary buffer, with every editing
command available — multiple cursors above all — then `Space t s`
(`:terminal-send [number|name]`) sends the selection to a terminal as one
bracketed paste, or the whole buffer when nothing is selected.

This is the only way modal editing can reach a program that owns its input
area, and it is what makes long prompts for a coding agent worth writing in the
editor.

**Example:** write a prompt for an agent running in terminal 2.

1. `Space b n` — open a scratch buffer, then `i` and write the prompt.
2. Escape — back to Normal mode, with nothing selected.
3. `:terminal-send 2` — paste the whole buffer into terminal 2.
4. Switch to the terminal and press Enter there to submit.

### Managing terminal sessions

A session outlives the pane showing it.

- `Space t q` shows the pane's buffer again and leaves the program running.
  Opening a file in the pane, or closing the split, does the same.
- When the program exits, a pane showing it returns to its most recently used
  buffer (or a scratch buffer) without closing. By default, the terminal
  session and its output are forgotten and disappear from the Finder and
  `Space t t`. Set `editor.auto_close_terminal` to `false` in `Space o o` to
  retain future exited sessions for review, search, or Close. Changing this
  setting does not discard sessions already retained.

#### The terminal list

`Space t t` (`:terminals`) lists sessions in the Navigator's columns (see
[Session and destination navigation](workspaces-and-persistent-sessions.md#session-and-destination-navigation)):

- running sessions first, then retained exited ones, dimmed; each group most recently
  activated first;
- `exited`, `unread`, and `bell` in the STATE column;
- the program, number, and directory in the preview.

**The preview** shows the live screen with its colours, updating while you move
through the list; an exited terminal shows its final screen. It is read-only
and never resizes the terminal. A wider screen is clipped, and a shorter
preview keeps the child's cursor row visible. `Ctrl-t` toggles it.

| Action | Effect |
| --- | --- |
| Enter | Visit: focus a pane already showing it, or show it here. An exited session opens in review. |
| **Show** (Tab) | Show the session in the active pane. The only action that attaches one. |
| **Bring into active pane** (Tab) | Move a session shown elsewhere to this pane; the old pane shows its buffer again. One PTY is never resized by two visible panes. |
| **Rename** (Tab) | Ask for a name and return to the list, leaving panes unchanged |
| **Close** (Tab) | End and forget the session. A hidden live process needs a second Enter. For an exited session, removes its output. |
| **Force kill** (Tab) | For a hung program. Always asks, naming the terminal, even when visible: Enter confirms, Escape cancels. Kills the process group, discards its output, and shows the pane's buffer. Unsaved work in the program is lost. Not offered for exited terminals. |
| **Create** (Tab) | Start a new terminal |

**Ending a program is always explicit:** type `exit` in it, or choose Close or
Force kill, or use `:quit-all!!` (`:qa!!`) to terminate this workspace’s terminal
processes, discard unsaved buffers, and quit. Ordinary quit spellings, including
`:qa!`, refuse while terminals run. Pending plugin writes and activity still
protect against quitting; finish those operations or stop their owner first.

**Other commands:**

- `Space t r` (`:terminal-rename <name>`) names the active session.
- `:terminal-show <number|name>` shows a specific session.
- Duplicate names are refused as ambiguous.

#### Numbers and titles

A pane title reads `[terminal #<number>] <name>`.

- The number is what `:terminal-send`, `:terminal-show`, and
  `:terminal-session-directory` accept. It is also in the list preview.
- Only running terminals have numbers. A new terminal takes the lowest free
  number, and gives it up when its program exits or it is closed, so numbers
  stay small and are reused. Numbers never depend on list order.
- Because a number can pass to a new terminal, it identifies what is running
  now. Do not record one in a macro or script expecting the same terminal
  later.
- A retained exited terminal has no number: its title is `[terminal] <name>`, and it
  is reached from `:terminals` or the Finder by name. Action menu titles use
  the `[terminal] <name>` form too.
- The name is the one you assigned, otherwise what the program calls itself:
  the title a shell sets from its prompt, or the program's name until it sets
  one.
- Scrolled back into history, the title adds `↑` and how far.
- The active pane adds `[insert]` while keys go to the child.

#### Lifetime

Sessions live as long as the workspace-host process that owns them:

| Survives | Does not survive |
| --- | --- |
| Normal detach | Force-stop |
| Client failure | Host replacement or crash |
| Switching workspaces | Logout or reboot |
| Reattaching | Machine failure |

`:detach` leaves persistent terminal children running without signalling them.
In ide and mux modes alike, ordinary quitting refuses while a terminal
runs; `:qa!!` explicitly terminates this workspace’s terminal processes.

### Emulation and limits

The emulator is Runyte's own, like the fuzzy scorer, the picker, and the diff.

**Supported:** colour including the 256-colour palette and true colour, the
usual attributes, scroll regions, insert and delete, the alternate screen,
bracketed paste, application cursor keys, cursor-position reports, and window
titles.

**Scrollback and performance:**

- Scrollback is bounded at 5,000 lines per session and by a measured 64 MiB
  retained-cell and review budget per workspace.
- Noisy sessions use independent bounded queues and round-robin byte and
  message budgets. PTY input is bounded and chunked.
- Terminal and review output is never written to disk.
- Output frames are coalesced to a bounded cadence. Repainting sends only
  changed rows, with a full resynchronization when the base is stale.
- Rapid identical wheel reports are merged into bounded requests, keeping
  their scroll distance and their order relative to clicks and keys.
- The alternate screen has no history, so while `htop` or `vim` runs there is
  nothing behind the screen to scroll to or copy.
- Inline TUIs on the primary screen may keep a composer or status area fixed
  while completed output scrolls through a top-anchored region. Those
  completed rows are ordinary scrollback, including Codex output in inline
  mode.

**Mouse.** SGR mouse reports are forwarded inside the pane body when the child
asks for them. Borders stay Runyte's, and the wheel scrolls review history
when the child has not asked for the pointer. In the native window,
Ctrl-left-click (Cmd-left-click on macOS) follows links instead, even while
the terminal is in input mode and requesting mouse reports.

**Colour queries.** Read-only `OSC 10;?` and `OSC 11;?` queries get the theme's
default foreground and background, so light- and dark-aware programs pick
fitting colours. A theme colour set to `reset` is unknown and gets no reply.
`OSC 4` palette queries and setters remain ignored even with theme ANSI colours
enabled: colour depth and palette rendering belong to each attached client.

**Known limitations:**

- Colour-setting and palette queries are ignored, as is `OSC 52`, so a program
  cannot change the palette or write your clipboard unasked.
- A cell keeps up to three combining marks without taking columns; more are
  deliberately dropped.
- Inline images — kitty graphics, sixel — are not passed through.
- Resizing does not reflow wrapped lines. Emulators disagree about what a
  resized wrapped line should become, and a wrong guess corrupts a live
  full-screen program worse than truncation does.
- Windows provides ide-mode and mux-mode ConPTY terminals.

<!-- navigation -->
[Contents](README.md) · [Previous: Panes](panes.md) · [Next: Git](git.md)
