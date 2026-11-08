## Terminals

A terminal session runs a program inside a pane: a shell, `htop`, `vim`, or a
coding agent. It is not a buffer, and it keeps running after the pane showing
it goes away.

### Starting a terminal

| Command | Runs |
| --- | --- |
| `:terminal` (`:term`, `:t`), `Space t n`, `Ctrl-w t` | `$SHELL` on Unix, `%COMSPEC%` (else `cmd.exe`) on Windows, in the editor working directory. Not available in [editor mode](workspaces-and-persistent-sessions.md#editor-mode). |
| `:terminal htop` | That command line |
| `:terminal-file-directory [command]` | From the active file's parent |
| `:terminal-directory-root [command]` | In the directory the active explorer shows |
| `:terminal-selected-directory [command]` | From the selected directory entry |
| `:terminal-session-directory <number\|name>` | A shell in another terminal's last safe directory |
| `Tab t` in an explorer | A terminal in the directory the explorer shows |

- The pane's own buffer stays where it is. Leaving the terminal shows it
  again.
- Unix splits command lines shell-style. Windows uses native
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
- Each terminal has its own process tree. Closing the terminal or quitting the
  editor ends that tree.
- Working directories need an ordinary Windows path shorter than 260 UTF-16
  units. Longer paths are refused before launch. `cmd.exe` also refuses UNC
  working directories, so use a shell that supports them. These limits don't
  affect which files the editor can open.

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

The title tells you whether typing reaches the program. NORMAL has no marker,
because the mode line already says it.

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
  aliases move to another pane at once. `Ctrl-w v/s` split, and the program
  keeps running in the original pane. The new pane opens the working-directory
  explorer, as `Space E` does. `Ctrl-w f/z` toggle full-screen and zen from
  terminal Insert, live Normal, Normal/review, and Select without changing the
  mode. Bare `Ctrl-w`, or cancelling the prefix, keeps the current mode.
  Window actions never capture or discard a review snapshot.
- **Arriving at a pane.** However you focus a live terminal (these keys, pane
  cycling, a mouse click), it enters Insert mode. A terminal with a captured
  review stays in review until `i`, `a`, `o`, or another insert key. A document
  reached from Terminal Insert starts in Normal.
- **`Ctrl-\` spelling.** On macOS, Runyte asks for unambiguous Ctrl-key reports
  without repeat and release events. A terminal without that protocol sends
  old-style control bytes, and `Ctrl-\` arrives as `Ctrl-4`. Both work.

#### Review mode

**Review navigates and copies. It does not edit.** The screen is only a picture
of the program's text, and the program owns its own input area. Commands that
would edit are refused.

A second `Ctrl-\`, or the first review command used from live Normal, freezes
the output as a snapshot. New output keeps arriving behind it.

- Motions move a visible review caret and keep it within the viewport margin.
- `v` enters Select mode. Motions then extend the selection, including both
  the start and end characters, in either direction. `Escape` cancels a
  selection.
- The title marks review and newer output, and the reviewed text is gray. A
  frozen image never looks like a live program.
- Moving to another pane does not start review. Returning to a reviewed
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
- `u` works only while the paste is still the program's last input. Any key
  typed into the terminal ends that. It is refused if the paste ended a line
  the program has already run. A shell at a prompt erases exactly the paste.

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
an ordinary read-only buffer. That is real text, so search, multiple
selections, `n`/`N`, and yank all work. `Ctrl-o` or `Alt-o` returns to the
terminal, and `Ctrl-i` or `Alt-i` goes back to the output.

**Into a terminal.** Write the text in an ordinary buffer, using any editing
command you like. Then `Space t s` (`:terminal-send [number|name]`) sends the
selection to a terminal as one bracketed paste. With nothing selected, it sends
the whole buffer.

This is how you use modal editing with a program that owns its input area. It
is very useful for writing long prompts for coding agents.

**Example:** write a prompt for an agent running in terminal 2.

1. `Space b n` — open a scratch buffer, then `i` and write the prompt.
2. Escape — back to Normal mode, with nothing selected.
3. `:terminal-send 2` — paste the whole buffer into terminal 2.
4. Switch to the terminal and press Enter there to submit.

### Managing terminal sessions

A session outlives the pane showing it.

- `Space t q` shows the pane's buffer again and leaves the program running.
  Opening a file in the pane, or closing the split, does the same.
- When the program exits, the pane stays open and shows its most recently used
  buffer (or a scratch buffer). By default, the terminal session and its output
  are forgotten and disappear from the Finder and `Space t t`.
- To keep exited sessions for review, search or Close, set
  `editor.auto_close_terminal` to `false` in `Space o o`. Changing this setting
  does not discard sessions you already kept.

#### The terminal list

`Space t t` (`:terminals`) lists sessions in the Navigator's columns (see
[Session and destination navigation](workspaces-and-persistent-sessions.md#session-and-destination-navigation)):

- running sessions first, then retained exited ones, dimmed; each group most recently
  activated first;
- `exited`, `unread`, and `bell` in the STATE column;
- the program, number, and directory in the preview.

**The preview** shows the live screen with its colours and updates as you move
through the list. An exited terminal shows its final screen. The preview is
read-only and never resizes the terminal. A wider screen is clipped, and a
shorter preview keeps the program's cursor row visible. `Ctrl-t` toggles it.

| Action | Effect |
| --- | --- |
| Enter | Visit: focus a pane already showing it, or show it here. An exited session opens in review. |
| **Show** (Tab) | Show the session in the active pane. The only action that attaches one. |
| **Bring into active pane** (Tab) | Move a session shown elsewhere to this pane; the old pane shows its buffer again. One PTY is never resized by two visible panes. |
| **Rename** (Tab) | Ask for a name and return to the list, leaving panes unchanged |
| **Close** (Tab) | End and forget the session. A hidden live process needs a second Enter. For an exited session, removes its output. |
| **Force kill** (Tab) | For a hung program. Always asks, naming the terminal, even when visible: Enter confirms, Escape cancels. Kills the process group, discards its output, and shows the pane's buffer. Unsaved work in the program is lost. Not offered for exited terminals. |
| **Create** (Tab) | Start a new terminal |

**Runyte never ends a program unless you tell it to.** You can:

- type `exit` in it
- choose Close or Force kill
- use `:quit-all!!` (`:qa!!`) to end this workspace's terminal processes,
  discard unsaved buffers and quit

Ordinary quit commands, including `:qa!`, refuse while terminals run. Pending
plugin writes and activity also block quitting. Finish those operations or stop
their owner first.

**Other commands:**

- `Space t r` (`:terminal-rename <name>`) names the active session.
- `:terminal-show <number|name>` shows a specific session.
- Duplicate names are refused as ambiguous.

#### Numbers and titles

A pane title reads `[terminal #<number>] <name>`.

- The number is what `:terminal-send`, `:terminal-show`, and
  `:terminal-session-directory` accept. It is also in the list preview.
- Only running terminals have numbers. A new terminal takes the lowest free
  number and gives it up when its program exits or it is closed. Numbers stay
  small and get reused. They never depend on list order.
- A number can pass to a new terminal, so it only identifies what is running
  now. Don't record one in a macro or script and expect the same terminal
  later.
- A retained exited terminal has no number. Its title is `[terminal] <name>`,
  and you reach it from `:terminals` or the Finder by name. Action menu titles
  use the `[terminal] <name>` form too.
- The name is the one you gave it. Otherwise it is what the program calls
  itself: the title a shell sets from its prompt, or the program's name.
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

`:detach` leaves persistent terminal programs running. In both ide and mux
modes, ordinary quitting refuses while a terminal runs. `:qa!!` ends this
workspace's terminal processes.

### Emulation and limits

The emulator is Runyte's own, like the fuzzy scorer, the picker, and the diff.

**Supported:** colour including the 256-colour palette and true colour, the
usual attributes, scroll regions, insert and delete, the alternate screen,
bracketed paste, application cursor keys, cursor-position reports, and window
titles.

**Scrollback and performance:**

- `terminal.scrollback_lines` keeps up to 2,000 history rows per terminal by
  default, not counting the visible screen. You can set it from 0 to 100,000 in
  the config or settings page. With 0 you get no history, but you can still
  review and copy the visible screen. Reloading the config or saving the
  setting updates all terminals. Lowering it drops the oldest history right
  away. Raising it can't bring discarded rows back.
- Each terminal can keep one frozen review snapshot, separate from the history
  limit. New output, other terminals and history-limit changes don't touch its
  text or selections. Returning to input releases it. A Finder result outside
  the snapshot can't replace it, so return to input before opening newer output
  for review.
- There is no shared memory budget for terminals. Memory grows with the number
  and width of terminals, retained history and review snapshots.
- Noisy sessions have their own bounded queues and take turns with byte and
  message budgets. PTY input is bounded and chunked.
- Terminal and review output is never written to disk.
- Output frames are combined to limit how often the screen updates. Repainting
  sends only changed rows, with a full refresh when needed.
- Rapid identical wheel reports are merged. They keep their scroll distance and
  their order relative to clicks and keys.
- The alternate screen has no history. While `htop` or `vim` runs there is
  nothing behind the screen to scroll to or copy.
- Inline TUIs on the primary screen may keep a composer or status area fixed
  while finished output scrolls above it. Those finished rows are ordinary
  scrollback, including Codex output in inline mode.

**Mouse.** SGR mouse reports are forwarded inside the pane body when the
program asks for them. Borders stay with Runyte. The wheel scrolls review
history when the program has not asked for the pointer. In the native window,
Ctrl-left-click (Cmd-left-click on macOS) follows links instead, even when the
program is requesting mouse reports.

**Colour queries.** Read-only `OSC 10;?` and `OSC 11;?` queries get the theme's
default foreground and background, so programs can adapt to light and dark
themes. A theme colour set to `reset` is unknown and gets no reply. `OSC 4`
palette queries and setters are ignored, even with theme ANSI colours enabled.
Colour depth and palette rendering belong to each attached client.

**Known limitations:**

- Colour-setting and palette queries are ignored, as is `OSC 52`. A program
  can't change the palette or write to your clipboard.
- A cell keeps up to three combining marks without taking columns. Any more are
  dropped.
- Inline images (kitty graphics, sixel) are not passed through.
- Resizing does not reflow wrapped lines. Emulators disagree about what a
  resized wrapped line should become, and a wrong guess corrupts a live
  full-screen program.
- Windows provides ide-mode and mux-mode ConPTY terminals.

<!-- navigation -->
[Contents](README.md) · [Previous: Panes](panes.md) · [Next: Git](git.md)
