## Commands

### The command palette

`:` opens the command palette: every command with its aliases, usage, and a
short description, grouped by category.

Exact names and aliases rank first, followed by name or alias prefixes, then
matches in descriptions and other metadata. For example, `:t` and `:term`
offer `:terminal` first, while `:tu` offers `:tutorial` first.

| Key | Action |
| --- | --- |
| Type | Filter by name, alias, description, or category |
| Up / Down | Select a result |
| Tab | Complete it |
| Enter | Run it |
| Escape | Close the palette |

- Commands whose service is unavailable for the active buffer stay listed but
  dimmed, with a reason. Running one leaves the typed command and editor state
  alone.
- `::` opens the plugin-only palette; see [Plugins](plugins.md#plugins).

#### Path arguments

Once a command taking a path is selected, the rows become filesystem hints,
from the editor working directory or from the absolute path being typed.

- Directories come first, with a trailing separator. Completing one keeps the
  palette open for its children. Running `:open` on a directory opens the
  explorer.
- `~` means your home directory, so `:open ~/.bashrc` and `:open ~/projects`
  work without a shell.
- Dotfiles are offered when the name typed begins with `.`, or when hidden
  files are enabled.

**Enter in path prompts** behaves the same everywhere: the palette's path
arguments, the `Space / p` finder path, and plugin fields that complete local
paths.

| Typed path | Enter |
| --- | --- |
| Does not name an existing file or directory, with hints showing | Accepts the selected hint, like Tab |
| Names an existing file or directory | Submits |

- `:cd sr` Enter Enter completes to `src/`, then changes into it.
- A new name that is a prefix of an existing one is completed too:
  `:w notes` beside `notes.md` becomes `:w notes.md`. Hints follow only a
  cursor at the end of the line, so press Left, then Enter, to submit `notes`
  as typed.
- Completion inside a buffer — words, language-server items, paths — still
  uses only Tab; Enter there inserts a newline.

#### The working directory

The working directory starts where Runyte was launched.

- `:cd <path>` changes it. Relative paths resolve from the current working
  directory. With an explorer active, `:cd` also retargets that explorer; from
  a file, the file stays open.
- `Space E` opens the working directory. `Space e` opens the active buffer's
  directory and selects its file, so Enter returns to it; from a rendered
  Markdown page it uses the source document. A pathless buffer falls back to
  the working directory.
- `Space f` (or `Space / f`) always opens the project Finder in name mode, and
  `:fuzzy-grep` its content mode.
- `:file-picker-directory` and `:fuzzy-grep-directory` search the active
  file's parent, the active explorer's root, or the working directory for a
  pathless or generated buffer.

### Command list

#### File and buffer commands

| Command | Aliases | Effect |
| --- | --- | --- |
| `:open <path>` | `e`, `edit` | Open a file or directory in the active pane |
| `:write [path]` | `w`, `save` | Save, optionally choosing a path |
| `:write! [path]` | `w!`, `save!` | Save, replacing an existing file or one that changed on disk |
| `:write-buffer-close` | `wbc` | Save and close the buffer in place |
| `:close` | `c`, `buffer-close`, `bc`, `close-buffer`, `cb` | Close the active buffer in place |
| `:close!` | `c!`, `buffer-close!`, `bc!` | Discard unsaved text and close the active buffer |
| `:buffer-new` | `new` | Open a new scratch buffer in the current pane |
| `:reload` | | Reload the active local/provider file, or refresh the explorer or supported Git list |
| `:path` | | Show the active buffer's absolute path in a wrapped popup; Tab offers copying it to the system clipboard (`s`) or the unnamed Runyte register (`r`) |
| `:cd <path>` | | Change the working directory; retarget an active explorer |
| `:explorer [path]` | `files` | Open an editable directory explorer |
| `:preview` | — | Native document preview of selected text or the complete buffer |
| `:render` | `markdown` | Render a Markdown document, or return to its source |
| `:diff-disk` | | Compare a fresh disk snapshot with the active file buffer |
| `:diff-remote` | | Compare a fresh remote snapshot with the active provider document |
| `:diff-this` | `difft`, `dt` | Mark this buffer, or compare it with the one marked before it |
| `:diff-off` | `do` | Close the comparison this buffer is part of |
| `:pipe <shell-command>` | `\|` | Replace each selection with the command's stdout |
| `:pipe-cancel` | | Cancel the workspace's running pipe job |

#### Finder commands

| Command | Effect |
| --- | --- |
| `:file-picker` | Open the Finder over files, buffers, and terminals |
| `:file-picker-directory` | Fuzzy-find below the active file/explorer directory |
| `:open-explorer-finder` | Open the Finder at the active explorer's directory |
| `:open-explorer-system` | Open the explorer's directory in the system file manager |
| `:fuzzy-grep` | Open the Finder in content mode |
| `:fuzzy-grep-directory` | Fuzzy-search contents below the active file/explorer directory |
| `:outline` (`document-outline`) | Open the Tree-sitter document outline |

#### Pane and view commands

| Command | Aliases | Effect |
| --- | --- | --- |
| `:vsplit [path]` | | Create a side-by-side split |
| `:hsplit [path]` | `split` | Create a stacked split |
| `:window-close` | `wc` | Close the active pane, but not the last one |
| `:resize-right +/- N` | | Grow or shrink the pane at its right edge by N cells |
| `:resize-left +/- N` | | Grow or shrink the pane at its left edge by N cells |
| `:resize-top +/- N` | | Grow or shrink the pane at its top edge by N cells |
| `:resize-bottom +/- N` | | Grow or shrink the pane at its bottom edge by N cells |
| `:zen` | | Toggle a centred, maximized editable writing viewport |
| `:fullscreen` | | Toggle the active pane across the whole editor area, at its ordinary width |

#### Quit and session commands

| Command | Aliases | Effect |
| --- | --- | --- |
| `:quit` | `q` | Close the pane and its unique buffer, or stop safely from the last one |
| `:quit!` | `q!` | Discard its unique buffer, or force quit from the last pane |
| `:quit-all` | `qa` | Quit safely regardless of pane count, without ending terminals |
| `:quit-all!` | `qa!` | Discard buffer changes and quit, without ending terminals |
| `:quit-all!!` | `qa!!` | Discard buffer changes, terminate this workspace’s terminals and quit |
| `:quit-here` | `qh` | Quit and return the shell to the active directory |
| `:quit-here!` | `qh!` | Discard changes, quit, and return there |
| `:write-quit` | `wq` | Save, then close the pane or quit from the last one |
| `:detach` | | Disconnect this persistent TUI while keeping all editor state |
| `:session-list` | `sl` | Open the session manager (Unix mux mode or Windows native controls) |
| `:session-attach WORKSPACE` | `attach` | Attach to another workspace's persistent session |
| `:session-stop [WORKSPACE]` | | Stop a clean persistent session |
| `:session-rename WORKSPACE NAME` | | Rename a persistent session |
| `:session-clean` | | Clean verified stopped session history (Windows) |

#### Terminal commands

| Command | Aliases | Effect |
| --- | --- | --- |
| `:terminal [command]` | `t`, `term` | Run a program in this pane, or `$SHELL` |
| `:terminal-file-directory [command]` | | Run from the active file's parent |
| `:terminal-directory-root [command]` | | Run from the active explorer root |
| `:terminal-selected-directory [command]` | | Run from the selected directory entry |
| `:terminal-session-directory <number\|name>` | | Run a shell from another terminal's safe directory |
| `:terminals` | | List the running terminals and show one here |
| `:terminal-show <number\|name>` | | Show a terminal in this pane |
| `:terminal-rename <name>` | | Name this pane's terminal |
| `:terminal-output` | | Copy this terminal's output into a read-only buffer |
| `:terminal-send [number\|name]` | | Send the selection, or the whole buffer, to a terminal |

#### Git commands

| Command | Effect |
| --- | --- |
| `:git-status` | Open the changed-file list |
| `:git-diff` | Show the active file's unstaged diff |
| `:git-diff-side-by-side` | Compare the active file's complete Git versions |
| `:git-index` | Review everything staged for the next commit |
| `:git-stage` | Stage the active file, or every file selected in the list |
| `:git-unstage` | Unstage the active file, or every file selected in the list |
| `:git-stage-hunk` | Stage the exact hunk under the cursor |
| `:git-unstage-hunk` | Unstage the exact staged hunk under the cursor |
| `:git-stage-lines` | Stage the supported saved source-line selection |
| `:git-discard` | Throw away a file's uncommitted changes, after a confirmation |
| `:git-commit` | Write a message and commit what is staged |
| `:git-branches` | Open the local and cached remote branch list |
| `:git-fetch-branch` | Fetch the selected cached remote branch or local upstream |
| `:git-merge-branch` | Review merging the selected branch into Current |
| `:git-conflicts` | Open unresolved index stages |
| `:git-merge-continue` | Review the resolved index and open a separately owned merge message |
| `:git-merge-abort` | Review aborting the active merge |
| `:git-worktrees` | Open the repository worktree list |
| `:git-compare` | Compare committed tips with the selected branch or worktree |
| `:git-log` | Open the Git log, or refresh it from its first page |
| `:git-network` | Open or refresh the cached commit network |
| `:git-network-head` / `:git-network-all` | Restrict the network to HEAD / all cached refs |
| `:git-network-choose-ref` | Choose a captured full ref |
| `:git-network-ref <full-ref>` | Restrict the network to an exact full ref |
| `:git-search-commits` | Fuzzy-search commits by message, ID, author, or date with a full-message preview |
| `:git-blame` | Show live-buffer attribution for the primary line |
| `:git-blame-file` | Open full-file live-buffer attribution |
| `:git-stashes` | Open or refresh the bounded stash list |
| `:git-stash-tracked <name>` | Stash a tracked-worktree snapshot, leaving the index applied |
| `:git-stash-all <name>` | Stash tracked worktree and index changes, after a confirmation |
| `:git-stash-untracked <name>` | Stash tracked changes and untracked files together |
| `:git-stash-apply` | Apply the selected stash without dropping it, after a confirmation |
| `:git-stash-drop` | Drop the selected stash, after a confirmation |
| `:git-refresh` | Refresh Git state or retry failed repository discovery |
| `:git-cancel` | Stop the active Git operation and reconcile mutations |

#### Language server commands

| Command | Aliases | Effect |
| --- | --- | --- |
| `:format` | `fmt` | Format the active buffer |
| `:lsp-trust` | | Choose workspace permission to run language servers |
| `:lsp-restart [language]` | | Restart stopped language servers |
| `:lsp-status` | | Report language server state |

#### Configuration, help, and diagnostic commands

| Command | Aliases | Effect |
| --- | --- | --- |
| `:config` | `settings` | Open the settings menu |
| `:config-reload` | | Re-read the loaded configuration file into this session |
| `:write-config [path]` | | Write the effective configuration to a new file, by default the per-user configuration file |
| `:write-config! [path]` | | Write the effective configuration, replacing any existing file |
| `:theme [name]` | | Choose a theme in the settings menu, or switch straight to the named one |
| `:help [topic]` | `?` | Open the general manual, optionally at a named section |
| `:about` | | Show Runyte's logo, version, and getting-started guide |
| `:tutorial [reset\|sessions\|next]` | | Open or resume the tutorial; `reset` starts over, `sessions` opens its persistent-session lesson, and `next` continues a completed exercise |
| `:grammar [runyte]` | | Report the active Runyte editing grammar |
| `:notifications` | `not` | Open retained notification history |
| `:service-health` | `health` | Inspect syntax, LSP, providers, and helper health |
| `:log-open` | | Open the diagnostic log owned by the process that holds this workspace |
| `:mcp [identity]` | | Review, grant, inspect, or revoke agent context access |

#### Plugin commands

| Command | Effect |
| --- | --- |
| `:plugins` | Open the configured plugin manager |
| `:plugin-stop <id>` | Stop a configured plugin or cancel its pending restart |
| `:plugin-restart <id>` | Restart a configured plugin after its cleanup finishes |

<!-- navigation -->
[Contents](README.md) · [Previous: Workspaces and persistent sessions](workspaces-and-persistent-sessions.md) · [Next: Plugins](plugins.md)
