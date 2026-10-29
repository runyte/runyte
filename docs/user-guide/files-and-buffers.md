## Files and buffers

### File and buffer keys

| Key | Action |
| --- | --- |
| `Space f` or `Space / f` | Open the Finder over files, buffers, and terminals by name or content; `Tab` switches modes and `Ctrl-t` toggles preview |
| `Space / a` | The same Finder over every file, ignore files not consulted |
| `Space / p` | The same unfiltered Finder rooted at a typed path, inside the workspace or outside it |
| `Tab f` in a directory | The same Finder rooted at the directory the explorer shows (`:open-explorer-finder`) |
| `:file-picker-directory` | Fuzzy-find a file or directory below the active file/explorer directory |
| `:fuzzy-grep-directory` | Fuzzy-search contents below the active file/explorer directory |
| `Space / /` | Search the workspace with a regular expression; see [Search](key-bindings.md#search) |
| `g f` | Open the selected path or web link, or the complete target under the cursor; relative files are matched beside the active file/explorer and at the project root |
| `Space e` | Open the active buffer's directory as an editable explorer; from a file, select that file |
| `Space E` | Open the working directory (controlled by `:cd`) as an editable explorer |
| `Space d t` / `Space d d` | Toggle the directory tree / reveal the active file in it |
| `Space b b` | Open the buffer list; Enter visits, `Ctrl-t` toggles the preview and `Tab` shows valid actions |
| `Space b c` | Close the active buffer safely (`:close`, `:c`) without changing the pane layout |
| `Space b d` | Compare a fresh immutable disk revision with the active file buffer (`:diff-disk`) |
| `Space b n` | Open a new scratch buffer in the current pane (`:buffer-new`, `:new`) |
| `Space r` | Reload the active text file or refresh the active explorer or supported Git list |
| `Space c y` / `Space c p` / `Space c P` | System clipboard yank / replace the selection, or paste after a bare caret / paste before |
| `?` in a Markdown document or the scratchpad | Render it as formatted text, or return from that page to the source (`:render`) |
| `Ctrl-s`, `:write`, or `:save` | Save |

Explorer keys are listed under [Explorer keys](files-and-buffers.md#explorer-keys), and tree keys
under [Directory tree sidebar](files-and-buffers.md#directory-tree-sidebar).

### Opening files and links

`g f` opens what is under the cursor, or exactly what is selected.

| Target | Opens |
| --- | --- |
| `https://…`, `http://…`, `www.…` | The default browser. `www.` addresses use HTTPS. |
| A file path | The file in the editor. Relative paths are matched beside the active file or explorer and at the project root. Several matches open a picker. |
| A Markdown link or image label | Its destination, including paths in angle brackets, in the source and in rendered `?` pages |
| A link to `#heading` or `file.md#heading` | That heading |

**Inferring the target** with a bare caret:

- Surrounding Markdown wrappers and trailing prose punctuation are left out.
  A path at the end of a sentence ignores trailing punctuation when the
  literal name does not exist.
- An inferred target stays within one buffer line.
- An explicit selection is used exactly.

**In terminal review** (`g f` in NORMAL/review mode):

- It reads the frozen review text, and resolves relative paths against the
  terminal's latest validated directory (or its launch directory) and the
  project root.
- Opening a file leaves the terminal process running. Opening a link keeps the
  terminal view in place.
- A bare caret anywhere on a web link follows the whole URL across rows joined
  by the terminal's automatic wrapping.
- An explicit newline ends a link, unless a program such as Codex or Claude
  Code broke the link inside indented text. The link then continues onto the
  next row when that row is indented to the same text column and the link's
  row reaches the right edge its block wraps at. A link ending before that
  edge, or a row starting at the left edge, is never joined.
- Resizing the live terminal clears wrap information. An already captured
  review keeps its original links.

**Opening by command:** `:open <path>` (aliases `:e`, `:edit`) opens a file or
directory in the active pane; see [Commands](commands.md#commands) for path completion.

### Desktop edition

Build the desktop edition with `cargo build -p runyte-desktop`.
Launch `target/debug/runyte-desktop --window` in a workspace, or add `--editor` to work
outside one. All existing editor keys and configured keymaps are dispatched by
the same registry. The interface remains the editor's cell grid, without menus
or toolbars. Window resizing changes the pane geometry and PTY dimensions.
In standalone mode the window close button requests `:qa`, preserving
unsaved-buffer and running-terminal refusal. In `--window --mux` it detaches,
leaving all editor state and terminal sessions in the host.

If the host cannot keep up and the bounded input queue fills, the window shows
“Some input was lost. Click to dismiss.” The notice remains until clicked.
Consecutive queued drags coalesce to their latest position; keys, clicks and
paste keep their order.

For the Runyte application icon on Wayland, register the desktop entry with
`python3 contrib/packaging/package.py linux --binary target/debug/runyte-desktop` before
opening a new window. X11 also receives an icon directly. A macOS `.app`
packaging helper and the platform limitations are described in the
[native desktop integration guide](../../contrib/packaging/README.md).

PNG, JPEG, GIF, WebP, BMP, and PDF paths open as read-only media projections in
the destination pane, including from `:open`, the explorer, the directory tree,
and startup filenames. Other binary types retain the external-program prompt.
Media never becomes an editable source file or a save target. Ordinary pane,
buffer, jump, and close commands remain available. Titles identify PDFs with
`[pdf]` and images with `[image]`. Images initially fit the pane while preserving
aspect ratio. Zoom, pan, and selection belong to each pane, so split views of
the same file can differ.

Animated GIF and WebP images play automatically while visible, using their frame
delays and loop counts. A finite animation stops on its last frame. Selecting a
region pauses that pane on the displayed frame; `y` / `Ctrl-c` copies from that
frame. Clearing the selection resumes playback. Split panes have independent
playback and selection state. Hidden previews and minimized windows do not keep
an animation timer running; restoring a preview resumes without replaying missed
frames. There are no separate pause, step, or restart bindings.

Frame delays below 20 ms use 100 ms to prevent continuous redraws. Decoding is
limited to 1024 frames and 32 MiB of retained raster data per animation, after
reduction to 2048 pixels per axis. An animation exceeding either limit shows a
clear error in its pane. For example, two 2048×2048 RGBA frames fill the 32 MiB
budget. Animated WebP EXIF metadata is limited to 64 KiB. Single-frame files
remain still images.

PDFs have a read-only **page buffer** with one row per page. Once the displayed
page is at fit size and has no selection, Escape reveals this buffer at the
current page. Use normal editor motions,
counts, or buffer search (`s` or `/`) to choose a row, then Enter displays that
page. Moving the cursor in the page buffer does not display pages until Enter.
`42gg` goes to page 42; `gg` and `ge`/`G` go to the first and last pages. Escape
or `q` from the page buffer opens the source directory with the PDF selected.
In Select mode, either key first clears the selection and returns to Normal mode.

Images have no page buffer: leaving their preview opens the source directory.
Escape first dismisses a pending overlay or key sequence. On zoomed-in media,
the next press returns to fit size in one step, preserving its selection;
subsequent presses clear the selection and then leave the media surface.
This fit-then-clear order applies to images and PDFs alike. `Space e` goes directly to
the source directory from either media type and selects the source file, just
as it does from a rendered Markdown document. `Space E` still opens the editor's
working directory.

In a PDF or image preview, `q` performs the same fit, clear-selection, and
leave-preview action as Escape, in both Normal and Select mode. Overlays
and pending key sequences retain their own bindings.

| Input in a media pane | Action |
| --- | --- |
| `+` / `=` / `-` | Zoom in / out |
| `z f` / `z 1` | Fit pane / actual raster pixels |
| `z h`, `z j`, `z k`, `z l` | Pan left, down, up, right |
| `h` / `l` | Pan horizontally |
| `j` / `k`, vertical arrows, counts | In Normal mode, pan a zoomed PDF vertically; change pages at fit size. Images always pan. |
| `Ctrl-f` / `Ctrl-d` / `Ctrl-n` / PageDown | Next PDF page at any zoom |
| `Ctrl-b` / `Ctrl-u` / `Ctrl-p` / PageUp | Previous PDF page at any zoom |
| `42gg` / `gg` / `ge` or `G` | Go directly to PDF page 42 / first page / last page |
| `z z` | Center the media |
| `v`, then `h/j/k/l` or arrows | Extend a PDF word/line selection or an image-region corner; `v` again stops extending |
| `%` | Select all page text, or the whole image when there is no text |
| `y` / `Ctrl-c` | Copy selected PDF text or selected image region to the system clipboard |
| Escape | Dismiss overlay or pending keys; then fit → clear selection → PDF page buffer → explorer. Images go directly from cleared selection to explorer. |
| Wheel / Shift-wheel | Pan vertically / horizontally; at fit size, vertical PDF scrolling changes pages |
| Ctrl-wheel (Cmd-wheel on macOS) | Zoom around the pointer |
| Middle-drag or right-drag | Pan |
| Alt-left-drag | Move the pane using the pane drop outline |
| Shift-Alt-left-drag | Extend the pane into neighbouring panes |
| Left-drag on PDF text | Select words in reading order |
| Left-drag on an image | Pan; double-click toggles fit / 2× fit |
| Shift-left-drag | Select a rectangular image region, including on PDFs |

There is no separate PDF page picker or `g p` binding. The paragraph commands
`goto-next-paragraph` and `goto-previous-paragraph` remain available for configured
keys without default bindings; `g p` and `g P` do not move between text paragraphs.

Scanned PDFs without embedded text use rectangular selection; the desktop edition
does not perform OCR. Copying a region exports the base page/image raster,
including alpha, without modifying the source file; PDF zoom refinement does
not increase the resolution of copied regions. PDF text selection uses Hayro glyph geometry and content-stream order.
If glyphs lack Unicode mappings or usable geometry, extraction independently
falls back to Poppler's `pdftotext`. If both fail, the page remains viewable and
region selection stays available. Complex layout reading order is heuristic;
this is not a full document-layout analysis.

Prompts and key hints remain readable above the media surface. Opening them
does not expose internal image placeholder rows or enter the PDF page buffer.

PDF support includes Hayro 0.8, run in a disposable helper process of the
same executable. Each page or refinement request automatically falls back to
separately installed Poppler tools when Hayro fails or reports unsupported
content. If both fail, the pane reports both errors. Hayro supports embedded
fonts and its standard Latin substitutes (including recognized nonstandard
font names); it does not consult system fonts. Substitution can change glyph
shapes and spacing. Other unavailable fonts require Poppler. Encrypted PDFs
with an empty user password can open; entering a PDF password is not supported.
Rendering runs on one background worker and reports failures inside the pane. Input is
limited to regular files up to 128 MiB, decoded images to 8192 pixels per axis
and 64 MiB allocation, and PDFs to 10,000 pages. Images are reduced to at most
2048 pixels per axis. PDF pages initially render at 1600 pixels on their longest
axis. When that is insufficient for the current zoom or display pixel density,
the visible region is rendered again in the background. The base page remains
visible while sharper text and vector graphics load. Each refinement is bounded
to 4096 pixels per axis and approximately eight million pixels; unusually large
or dense displays may still exceed that detail budget. Eight base page/image
results and eight visible-region refinements are cached separately using
least-recently-used eviction. The base cache is also bounded to 256 MiB, enough
for eight animations at their individual limit. GPU images no longer displayed
by any pane are released. Zooming, panning, resizing, or leaving a pane
cancels refinements that are no longer needed by any visible pane.
After a PDF page loads, up to two pages on either side are loaded in the
background. Requested pages take priority and interrupt speculative rendering.
First loads and jumps beyond the warmed pages can still show a loading message.
PDF subprocesses have a 15-second limit per invocation and are killed and reaped
during cancellation. Hayro additionally has a 1 GiB address-space budget
(on macOS, above the executable's initial mappings), a 15-second CPU limit,
and bounded RGBA output. Its normal path uses no raster temporary files.
Poppler fallback retains its temporary PNG/XHTML files and existing limits.

On Linux/macOS, `runyte --window --mux` creates or attaches to the same
persistent sessions as `runyte --mux`. Only one interactive frontend attaches
at a time. `:detach` closes the window and preserves the session; `:quit` from
the last pane and `:quit-all` stop a clean session and reuse the window for the
previously visited available running session, closing it when none is available.
Unsaved edits and live terminal children still refuse a stop. Session navigation
uses the same `Space Space`, `Space 1`–`Space 9`, `Shift-Left` / `Shift-Right`,
and `Ctrl-w a` bindings in either frontend.

Media panes survive frontend handoff. A terminal client retains their `[pdf]`
or `[image]` title and displays `Media viewing needs the Runyte window: use the desktop edition with --window` in the
body; Escape and `Space e` still reach the source directory. Opening a binary
while the terminal client is attached uses the external-program prompt. A
window attachment opens supported media in panes, with decoding and PDF
rasterization performed by the window process.

The window client needs the desktop edition; either edition can serve the
persistent host or attach as a terminal client. Every client, host, and
integrated `runyte --wait` must use the same Runyte version; a private protocol
mismatch is refused at the handshake. Integrated Claude Code and Codex `Ctrl-g` edits work through
this persistent host and survive detach and frontend handoff.

The native window supports `ide`, `editor`, and `mux` modes; host and lifecycle
modes are CLI operations. The desktop window is not available on Windows.
The window embeds JetBrainsMono Nerd Font in Medium, Medium Italic,
Bold, and Bold Italic. Set `editor.font_size` in `Space o o` (8–48 logical
pixels, default 15); the saved size is read when each window opens, including
reattachment to an existing persistent session. Reopen the window after saving
the setting. Ctrl-plus (also Ctrl-equals) and Ctrl-minus adjust the current
window by one pixel without writing the configuration. Text, pointer targeting,
panes and terminal dimensions scale together. No system font installation is required.

Ctrl-Shift-c copies the editor selection, terminal review selection, or selected
media to the system clipboard. Ctrl-Shift-v pastes clipboard text as literal input
into Insert/Replace mode, a prompt, or an integrated terminal, like a terminal
emulator's paste shortcut. Normal mode retains ordinary modal paste commands
(`Space c p` or Ctrl-v). Ctrl-c remains terminal child input. Native-feature
builds use an in-process clipboard owner for `Space c y`, Ctrl-v and the other
system clipboard commands; external clipboard helpers are not required.
Window-local font and literal-paste shortcuts are reserved frontend controls,
listed in contextual help; they do not participate in editor key remapping.
On macOS, Cmd-c also copies, and Cmd-v also pastes literal
clipboard text. Ctrl-Shift-c/v remain available. The Command modifier is
displayed as `Cmd` in macOS help and key hints. Configuration accepts both
`Cmd-c` and `Super-c` for the same key (and likewise for other Command keys),
on every platform. Other Command shortcuts are left unchanged.
Search within PDF page contents is not implemented.
Image zoom and scanned PDF content remain limited by their
source resolution. PDF text and vector graphics are rendered at the view
resolution within the visible-region budget described above. PDF rendering uses
a bundled Rust helper, with separately installed Poppler as fallback. Source changes
are checked when another frame is presented; there is no automatic media reload
watcher. macOS, Wayland, IME composition, and touchpad pinch gestures still need hands-on
platform verification. Ctrl-wheel zoom is supported; native pinch gestures are
not exposed by this GPUI release.

### Opening binary files

A file whose bytes contain a NUL, or do not decode as UTF-8, is binary.
Opening one asks which program should have it instead of loading it as text.

| Key | Effect |
| --- | --- |
| Up / Down | Choose an offered program |
| Enter | Open the file with it |
| Tab on a remembered choice | Delete it, or make it the default selection |
| Esc | Leave the file alone |

- **Offered programs.** The prompt first selects the desktop's preferred
  application: `xdg-open` on Linux, `open` on macOS, **System default** on
  Windows. Recent explicit choices follow, most recent first, and the most
  recent seeds the prompt.
- **Typing a program.** Type a name on `PATH` or an absolute path. Explicit
  choices are remembered.
- **How it runs.** The program starts detached with no terminal of its own:
  viewers and GUI applications work, but a terminal program cannot take over
  the screen.
- **Detection.** An initial 8 KB probe avoids reading an obvious binary twice.
  The final read still validates the whole file before it becomes a buffer.
- **Where choices are kept.** Runyte's platform cache directory:
  `$XDG_CACHE_HOME/runyte`, else `~/.cache/runyte` on Linux or
  `~/Library/Caches/runyte` on macOS. Without an explicit `XDG_CACHE_HOME` on
  Unix, `~` comes from the effective operating-system account, not the
  inherited `$HOME`, so a privileged launcher cannot put cache files in another
  account's home.

#### Opening binary files on Windows

- Applications open in the background while editing stays responsive.
- An explicit program is remembered only after Windows accepts its launch.
- A launch still unresolved after five seconds reports that uncertainty and is
  not retried. Acceptance does not prove the application displayed the file.
- Viewers can stay open after Runyte exits, subject to restrictions imposed by
  the process that started Runyte.
- An explicit program uses native executable lookup and argument quoting.
  Quote paths with spaces: `"C:\Program Files\Viewer\viewer.exe" --fit`. The
  file is passed as one final argument.
- Shell operators are literal arguments, and script wrappers are not found as
  native programs.
- The system-default handler needs an equivalent ordinary path shorter than
  260 UTF-16 units; an explicitly chosen viewer may support longer extended
  paths.
- Opening from network shares is not part of the validated support.

### Saving and closing

| Command | Effect |
| --- | --- |
| `Ctrl-s`, `:write`, `:w`, `:save` | Save the active buffer |
| `:write <path>` | Save as `<path>` |
| `:write!` (`:w!`, `:save!`) | Save, replacing an existing file or one that changed on disk |
| `:close[!]` or `:c[!]`, `Space b c` | Close the active buffer in place; `!` explicitly discards unsaved text; terminals are refused |
| `:write-buffer-close` (`:wbc`) | Save and close the buffer in place |
| `:window-close` or `:wc` | Close the active pane, but refuse the last pane |
| `:quit[!]` or `:q[!]` | Close the active pane and its uniquely displayed buffer; from the last pane, exit ide mode or stop the persistent session and return to a previous running session, with unsaved-change protection |
| `:quit-all[!]` or `:qa[!]` | Exit ide mode or stop the persistent session and return to a previous running session regardless of pane count, with unsaved-change protection; never terminate terminals |
| `:quit-all!!` or `:qa!!` | Discard unsaved buffers, terminate this workspace’s terminal processes, and quit or stop its persistent session; return to a previous running session when available |
| `:quit-here[!]` or `:qh[!]` | Quit and let the shell wrapper change to the active explorer/file directory |
| `:write-quit` (`:wq`) | Save, then close the pane or quit from the last one |

- Closing a buffer keeps every pane. Each pane that showed it returns to its
  own most recently used live buffer, or a scratch buffer when none remains.
- Quitting in mux mode is described under
  [Quitting and detaching](workspaces-and-persistent-sessions.md#quitting-and-detaching).
- Only `:quit-all!!` (`:qa!!`) ends terminals as part of quitting; see
  [Managing terminal sessions](terminals.md#managing-terminal-sessions).

### The Finder

`Space f` opens the Finder: one ranked list of files below the project root,
open buffers, and terminal sessions. In [editor mode](workspaces-and-persistent-sessions.md#editor-mode) it
covers the active file's or explorer's directory instead, and names it in the
title.

| Mode | Matches | Enter |
| --- | --- | --- |
| Name (default) | Names and paths | Opens the file for editing, a directory in the explorer, or the buffer or terminal |
| Content (`Tab`) | Lines in files, in-memory buffers (including pathless ones), and terminal scrollback plus the current screen | Opens the location; for a terminal, enters review at the matched row |

`Tab` switches modes without clearing the query. `:fuzzy-grep` opens content
mode directly. `:fuzzy-grep-directory` is a separate, files-only content search
rooted at the active file's or explorer's directory.

#### Finder keys

| Key | Action |
| --- | --- |
| Up / Down, `Ctrl-p` / `Ctrl-n`, paging, Home / End | Move |
| `Tab` | Switch name / content mode |
| `Shift-Tab` | Select the previous row |
| `Ctrl-t` | Toggle the preview |
| Enter | Open the selected result |
| `Ctrl-s` / `Ctrl-v` | Open a file result in a horizontal / vertical split (files only) |
| Escape or `Ctrl-c` | Close |
| `Space` with an empty query | Close |
| Backspace / Delete and prompt control keys | Edit the query |

- `q`, `j`, `k`, and other printable letters are query text, not commands.
- After a query has begun, `Space` separates terms. Every filterable list
  follows this rule; a report, which has no filter, always closes on `Space`.
- In other pickers (directory-scoped and fuzzy-content), `Tab` keeps moving to
  the next row and `Shift-Tab` selects the previous one.
- `Space b b` and `Space t t` remain the place for Save, Discard, Close,
  Rename, and other actions. The Finder only opens things.

#### Writing a query

| Query | Finds |
| --- | --- |
| `fpick` | `file_picker.rs` |
| `kmap validate` | `src/keymap/validate.rs` |
| `src picker` | `src/picker.rs` (but `picker src` finds nothing) |
| `docs/` | Directories only, matched without the slash |
| `term build` | Terminals first, then anything else matching `build` |
| `Readme` | Case-sensitive, because of the capital |

- **Subsequence.** A term matches when its characters appear in order.
  Exact basenames, basename prefixes, consecutive characters, and
  path-component boundaries rank highest.
- **Spaces** split the query into terms, which must appear in the typed order.
  Each term is as loose as a lone word, so `ab cd` accepts exactly what `abcd`
  does. What changes is ranking: distance between terms is free, while the
  same distance inside one term is a gap that lowers the score. Typing words
  apart says you expect them apart.
- **Highlighting.** Terms landing on contiguous spans are emphasized as a
  direct match; a term that had to spread out is shown in the secondary
  colour.
- **Smart case.** One capital anywhere makes every term case-sensitive. Both
  pickers and fuzzy grep use this rule.
- **Type hints.** `file`, `terminal`, `term`, and `buffer` move that kind
  first without hiding other matches.

**What name mode searches:**

- Files: absolute, project-relative, `~/`-relative, and basename spellings of
  their paths.
- Buffers: their structural name and path. A file that is open appears once,
  as the live buffer, so unsaved text and naming win over disk.
- Terminals: assigned name, child title, launch program, number, current
  reported directory, and initial directory.

#### Previews

`Ctrl-t` toggles the preview without changing the mode or query.

- A text file: its first 64 KiB, even when the file is larger.
- A directory: a one-level listing of its files and subdirectories.
- A buffer: its in-memory text.
- A terminal: the live screen in name mode; in content mode, a static numbered
  snippet around the matching row.
- A content match: surrounding lines filling the preview's height, resized
  with the overlay, keeping the matching line visible. Up to 512 context lines
  are kept, and the matching line shows its real line number.
- A direct content match (a single word on a contiguous span, or every term of
  a multi-word query on a span of its own) is filled with the primary match
  colour. A match with a gap inside a term fills its individual characters
  with the secondary colour.

#### What is scanned

- Regular files and directories, found natively: no `git`, `find`, `fd`, or
  external fuzzy finder.
- Nested `.gitignore` and `.ignore` rules apply, including ancestor rules when
  `:file-picker-directory` starts below the project root.
- Symlinks are never followed. `.git`, `.runyte`, and the configured workspace
  state directory are never scanned.
- `editor.show_hidden_files` applies.
- Content mode reads non-empty UTF-8 lines of ignore-aware files, shows
  `path:line`, and skips files larger than 4 MiB. Unsaved buffers replace
  their disk contents; pathless buffers and terminal output are searched
  directly.

**Content search runs inside the scan.** Editing the query restarts the walk,
which keeps only matching lines.

- The 50,000-candidate bound that keeps ranking fast limits matches, not how
  much of the project is read, so a match is found wherever it is.
- Typing more after a complete scan narrows what is already loaded.
- `result limit reached` in the title means more than 50,000 matches; a
  longer query resolves it. On a 150,000-line project only a single-character
  query reaches it.

**Responsiveness.** Typing never waits for discovery, ranking, sorting,
resource matching, or previews. The prompt updates on the next frame and a
progress label stays until the rows match the query. Rows from the previous
query may stay visible briefly, so the overlay does not flash empty, but they
cannot be opened until the new ranking arrives. Results for older queries are
discarded.

#### Wider scopes

| Finder | Ignore rules | Root | Title |
| --- | --- | --- | --- |
| `Space f` | Applied | Project root | (none) |
| `Space / a` | Off | Project root | `all files` |
| `Space / p` | Off | A typed path, inside the workspace or not | `all files in <path>` |
| `Tab f` in an explorer | Applied | The explorer's directory | That root's path, when below or outside the project root |

- With ignore rules off, build output, vendored trees, and generated code are
  reachable. Every other exclusion (`.git`, `.runyte`, the state directory,
  symlinks, `editor.show_hidden_files`) still applies.
- `Space / p` completes the path as you type: `~` expands, a relative path
  resolves against the working directory, and `Tab` accepts the selected row.
  Enter also accepts a row while the typed text is not an existing entry, and
  opens the Finder once it is.
- An explorer inside the project keeps the ignore rules from the project root
  down. One elsewhere reads ignore files from its own directory down. Dotfiles
  follow `editor.show_hidden_files`.
- The scope and title survive the `Tab` into content mode, so an ignored
  file's lines are searchable too.

### Buffer list

`Space b b` lists open buffers in the Navigator's columns (see
[Session and destination navigation](workspaces-and-persistent-sessions.md#session-and-destination-navigation)).

| Key | Action |
| --- | --- |
| Enter | Visit the buffer, focusing a pane already showing it |
| `Ctrl-t` | Toggle a bounded preview of its in-memory text |
| `Tab` | Actions, starting with **Bring into active pane** |

**Tab actions:**

- **Bring into active pane** shows the buffer here instead of focusing another
  pane.
- **Close hidden buffers** closes clean buffers not visible in any pane,
  keeping unsaved edits and pending saves. It applies to the whole workspace
  even when the list is filtered.
- A modified file offers **Save** and **Discard changes**. Discard asks for a
  separate Enter.
- **Close** appears only once a buffer is clean, never discards edits, and
  redirects every pane that shared the buffer.
- Explorer buffers offer only **Bring into active pane** and **Close hidden
  buffers**. Filesystem changes happen only by editing an explorer and
  confirming its `:write` plan.

**Retention of special buffers.** The eight most recently active clean
generated and special buffers, explorers included, are kept after their last
pane moves away. Activating a ninth retires the least recently used detached
one.

- This keeps `Ctrl-o`/`Ctrl-i` and `Alt-o`/`Alt-i` useful without letting
  `Space b b` or the Finder accumulate stale views.
- A dirty special buffer stays open until saved or discarded.
- A rendered Markdown page is not counted and stays while its document is
  open; it closes with the document.
- An empty, clean scratch buffer retires as soon as its last pane leaves,
  unless a pane is reading its rendered page. Written scratch text is an
  ordinary buffer.

### Directory buffers

Open a directory with `runyte <directory>`, `:open <directory>`, `Space e`, or
`Space E`. It becomes an editable buffer: one relative path per line, with `/`
after directories.

```text
docs/
src/
Cargo.toml
README.md
```

#### Editing entries

Edit the listing with ordinary modal editing and multiple selections:

| Edit | Filesystem change |
| --- | --- |
| Change a name | Rename |
| Delete a line | Delete |
| Add a line | Create a file |
| Add a line ending in `/` | Create a directory |
| Add `docs/roadmap.md` | Create a file inside an existing subdirectory |
| Change a path | Move |
| `x` then `y`, go to the destination, `p` | Copy |
| `x` then `d`, go to the destination, `p` | Move (cut) |

- **Copy and cut** work after navigating in the same pane or across split
  panes. The register keeps the filesystem identity, so pasted entries keep
  their contents and directories are copied or moved recursively.
- **A pasted cut** is applied by writing the destination explorer. Writing the
  source first is refused, because that would delete the move's source.
- **Copying within one directory:** rename the pasted row before writing.
- **Nothing happens until you write.** See [Applying changes](files-and-buffers.md#applying-changes).

**Rows that are not changes:**

- Trailing whitespace and whitespace-only rows are ignored. Writing a listing
  with only those edits refreshes it to the canonical view.
- A name ending in whitespace, containing a control character, or not valid
  UTF-8 cannot be told apart from the row syntax, so such a directory refuses
  to open as a listing. New rows are held to the same rule before a
  confirmation opens.
- A directory cannot be moved below itself: editing `dir/` into `dir/child`
  is rejected by `:w` before any confirmation.

#### Applying changes

`:w` or `:write` opens a plan listing every create, rename, move, copy, and
delete.

| Key | Effect |
| --- | --- |
| Enter | Apply, sending deletions to the operating-system trash |
| `P` | Apply with permanent deletion |
| Escape | Cancel |
| Arrows, paging, Home, End | Scroll the plan |

The confirmation shows each reviewed operation and the total.

**Before applying**, the whole plan is rejected if:

- the directory's visible entries changed on disk since it was opened;
- an entry the plan moves, copies, or deletes changed;
- anything changed below a directory the plan moves, copies, or deletes
  (their complete trees are recorded when the plan is prepared).

Activity inside an unaffected child directory does not stale the explorer.

**While applying:**

- On Linux, macOS, and Windows, a move, rename, or copy refuses to overwrite
  an entry that appeared after the plan was checked.
- Creation and rollback need an empty destination name, even when the
  competing entry is a dangling symlink.
- A filesystem without exclusive rename is refused for operations that need
  it; Runyte never falls back to an overwriting rename.
- If an operation fails midway, an ERROR notification names it and every
  operation already applied.

**After applying:**

- Other affected clean explorers refresh, and open files follow confirmed
  renames and moves.
- Affected explorers with unsaved edits are kept, with a warning.
- Deleted open paths are reported as stale, not silently redirected.
- The explorer that applied the plan keeps its edited row order, so a renamed
  or new entry stays where you wrote it. Entering it again later restores the
  sorted listing.

**What is not guaranteed.** These checks prevent destination collisions; they
do not make a plan atomic.

- Earlier operations, including confirmed deletions, may already have
  completed when a later one fails.
- Concurrent replacement of a parent or source entry, changes during
  recursive operations, and native trash path races are outside these
  guarantees.

#### Recovering from a failed plan

If rollback finds that an original name was recreated meanwhile, both entries
are kept.

- The ERROR notification gives the original path and the absolute location of
  the retained original inside a `.runyte-move-…` staging directory. Failed
  copy cleanup can likewise leave a `.runyte-copy-…` directory, also named in
  the notification.
- The notification buffer keeps these details after the interaction line
  moves on, within the notification history and size limits.
- Staging directories can hold the only remaining copy of a file. Runyte does
  not delete them on exit or when another plan runs.
- Inspect both entries before restoring the retained one to a free name, and
  refresh the explorer before preparing another plan.

#### Explorer keys

| Key | Action |
| --- | --- |
| Enter | Open the selected file or directory (the only key that opens; `e` stays the word-end motion) |
| `-` or Backspace | Open the parent directory and select the child just left |
| `r` or `Space r` | Refresh the listing |
| `.` | Show or hide dotfiles |
| `?` | Toggle read-only permissions, owner, group, size, and modification-time columns |
| `x y` / `x d`, then `p` | Copy / cut selected entries, then paste in the destination explorer |
| `:w` or `:write` | Review a filesystem plan before applying edits |
| `Tab` | Actions, including the ones below and the three display settings |
| `Tab t` | Open a terminal in this pane, starting in the shown directory |
| `Tab f` | Open the Finder rooted at the shown directory (`:open-explorer-finder`) |
| `Tab e` | Open the shown directory in the system file manager (`:open-explorer-system`) |
| `Tab s` | Open a persistent session here (see [Session and destination navigation](workspaces-and-persistent-sessions.md#session-and-destination-navigation)) |

`Tab t`, `Tab f`, and `Tab e` use the directory the explorer shows, not the row
under the cursor.

- **`Tab t`** keeps the explorer behind the terminal, with any unsaved edits.
- **`Tab e`** uses `xdg-open` on Linux and `open` on macOS. Unapplied edits
  stay in Runyte; the file manager sees the directory as it is on disk.

#### Display settings

| Setting | Toggle | Effect |
| --- | --- | --- |
| `editor.show_hidden_files` | `.` | List dotfiles or leave them out |
| `editor.explorer_details` | `?` | Show detail columns before each entry |
| `editor.explorer_sort` | (`Tab`) | The order rows appear in |

- `Tab` offers all three. They are ordinary settings, so `Space o o` reaches
  them too, and a choice here is written to `config.yaml`: a listing you set
  up once stays that way next session.
- A change re-reads every clean explorer, not only the active pane.
- If the value cannot be saved (no configuration file was loaded, or it uses a
  construct Runyte will not patch), it still applies to this session and the
  status line says it was not written.
- An explorer with unsaved edits refuses a change to dotfiles or order,
  because re-reading would discard them. Details only prefix existing rows, so
  `?` keeps working.

**Details columns** show permissions, owner, group, human-readable size, and
modification time.

- They are presentation only; the file name stays the only editable text.
- Entries from other years show the year instead of the time.
- In a narrow pane, horizontal scrolling moves through the details first, then
  the file name, so no column is permanently clipped.

**Sort orders.** Directories always come first.

| Value | Order |
| --- | --- |
| `name` / `name_descending` | A to Z / Z to A |
| `modified` / `modified_descending` | Oldest / newest first |
| `size` / `size_descending` | Smallest / largest first |

A directory's own size describes how its entries are stored, not how much it
holds, so a size order leaves directories in name order. Entries with equal
keys keep name order, so nothing shifts between redraws.

**Hidden entries and plans.** A listing without its dotfiles is planned
without them: an entry it never showed is not deleted for being absent, nor
reported when it appears. A new name colliding with a hidden one still stops
the plan instead of overwriting it.

**Symlinks** are listed under their own name, followed by a muted
`→ target` hint showing exactly what the link stores.

- The hint is not buffer text: it cannot be selected, edited, or written.
  Every hint in a listing starts in the same column.
- Enter opens what the link points at, so you edit the file Git and the
  language server know about. A broken link says so.
- Rename, copy, cut, and delete act on the link itself.

#### Navigating and splitting

- **One explorer per pane.** Each pane browses with one explorer, retargeted
  as it walks. `Space b b` lists one directory buffer per pane, labelled
  `[explorer] dirname` with its project-relative path (`.` for the project
  root).
- **Remembered rows.** A directory comes back on the row you left. Going to
  the parent with `-` or Backspace selects the child just left, so Enter
  returns to it. If that child is hidden by the dotfile filter, the parent's
  remembered row stays selected.
- **Unsaved edits.** Retargeting discards the listing, so refreshing or
  navigating away from unsaved edits asks first.
- **Splitting an explorer** works like splitting a file: the new pane shows
  the same listing on the same row. Navigate it elsewhere and it gets an
  explorer of its own, which is how copy and cut across two explorers work.
- **Closing an explorer** retires its buffer and keeps every pane. Each pane
  returns to its most recently used live buffer, then any live buffer, or a
  scratch buffer. `:c` refuses unapplied edits; `:c!` drops the plan without
  touching the filesystem.

#### Changes made outside Runyte

An explorer is watched like an ordinary file. When another process adds,
removes, or renames an entry directly in its directory, every pane showing it
gains `[STALE]`.

- The listing is left alone: rows, selections, unsaved renames, and undo
  history survive, since replacing them would discard edits not yet written.
- A clean explorer attached to no pane refreshes automatically when brought
  back into a pane. Unapplied edits are preserved and still require an explicit
  refresh.
- `Space r` or `:reload` re-reads the directory and clears the marker. So does
  navigating elsewhere or writing a plan.
- Entries hidden by the dotfile filter are not compared, so their changes go
  unnoticed too.

#### Names on Windows

- Reserved device names, alternate data stream syntax, trailing spaces or
  dots, and names differing only in case are rejected.
- Renaming one entry by changing its case is supported.
- Moves between volumes are refused before staging. Copy, check the copy, then
  delete the source.
- Quoted command paths keep literal backslashes, including UNC roots and a
  trailing separator: `:open "C:\work files\note.txt"`.

### Directory tree sidebar

The directory tree is a sidebar at the left of the editor area. It is not a
pane or a buffer.

| Key | Action |
| --- | --- |
| `Space d t` | Show or hide the tree |
| `Space d d` | Show and focus it, expand the active file's ancestors, and select the file |
| `Ctrl-w h` / `Ctrl-w l` | Move focus across the tree boundary (`Ctrl-h` / `Ctrl-l` with `editor.fast_pane_keys`) |
| Escape | Return to the previous pane, keeping the tree shown |

- `Space d d` refreshes those listings, so files created or moved outside the
  editor can be revealed. A file outside the workspace, or a buffer with no
  file, selects the workspace-root row.
- The `[dir tree]` title sits above the root row. The tree stays rooted at the
  workspace root when `:cd` changes the working directory.
- Expansion, selection, scroll position, legend visibility, and a resized
  width survive hiding it.
- Fullscreen and zen cover it temporarily. Terminals narrower than 36 columns
  hide it.

#### Moving in the tree

| Key | Action |
| --- | --- |
| `j` / `k`, Down / Up | Next / previous row |
| Home / End, `gg` / `ge` or `G` | First / last row |
| PageUp / PageDown, `Ctrl-b` / `Ctrl-f` | Page up / down |
| `Ctrl-u` / `Ctrl-d` | Half a page |
| `gt` / `gc` / `gb`, `H` / `M` / `L` | Top / middle / bottom visible row |
| `l` or Right | Expand a directory, or select its first child |
| `h` or Left | Collapse a directory, or select its parent |
| Enter on a directory | Toggle expansion |
| `Space r` | Refresh the selected directory |
| Click | Select a row and focus the tree |
| Double-click | Act as Enter: toggle a directory or open a file |
| Wheel | Move the selection three rows |

Digits choose destination panes (below), not counts. A double-click is two
left presses on the same row within half a second; a third press starts a new
pair rather than toggling the directory straight back.

#### Tree actions

| Key | Action |
| --- | --- |
| `.` | Show or hide dotfiles in the tree. |
| `/` | Search visible entry names with a regular expression. |
| `gw` | Label onscreen entries; type a label to select its entry without opening it. |
| `Ctrl-n` / `Ctrl-p` | Select the next/previous tree search match, wrapping. |
| `n` | Create a file; end its relative path with `/` to create a directory. |
| `d` | Delete the selected entry after `Delete <path>? [y/N]` in the interaction line. |
| `m` | Move to a typed path; Tab completes paths in this prompt. |
| `r` | Rename the selected entry. |
| `v` | Open the selected file in a vertical split. |
| `s` | Open the selected file in a horizontal split. |
| Enter | Open the selected file in an existing pane. |
| `1`–`9` | Open the selected file directly in that numbered pane. |
| Tab | Toggle the dimmed key legend below the horizontal rule. |

**Choosing a pane.** With one pane, Enter, `v`, and `s` act at once. With
several:

1. Pane titles are replaced by numbers from 1, top to bottom and left to
   right. The tree is not numbered.
2. Type a digit to open in that pane, or split it. With more than nine panes,
   type a longer number; Enter accepts a number that is also a prefix of
   another.
3. Escape cancels and restores the titles.

Numbers are recalculated for each request.

**Changing files.** Create, move, and rename apply as soon as the prompt is
submitted; Escape cancels the prompt, and there is no review stage. Delete
needs `y`; Enter, `n`, or Escape cancels. Deletion goes to the trash first.
Every operation rechecks filesystem identities and destination collisions,
refuses to overwrite, and updates affected open buffers. The safety and
non-atomicity limits of [Applying changes](files-and-buffers.md#applying-changes) apply.

**Jump labels** replace the markers before names: one key for nearby entries,
two for farther ones. Only rows visible above the legend are labelled. Escape
or an unmatched key cancels and keeps tree focus.

**The legend** is shown by default and wraps to the tree width: `n: new`,
`d: delete`, `m: move`, `r: rename`, `v: open in v-split`,
`s: open in h-split`, `Tab: legend`, `.: hidden files`, `/: search`. Remapped
keys show their live spellings.

**Hidden files.** The tree starts with `editor.show_hidden_files`. The `.`
override belongs to this workspace's tree and survives hiding it.

- Hiding a selected dotfile, or a hidden ancestor, selects the nearest visible
  ancestor.
- Revealing a file explicitly can still show a hidden path; toggling
  visibility clears that exception.

**Tree search.** `/` opens `tree search (regex):`.

- Enter selects the next matching visible name. Matching ignores case unless
  the pattern uses `(?-i)`.
- Collapsed directories are not searched.
- An empty prompt repeats the previous search, and Escape cancels without
  moving.
- `Ctrl-n` / `Ctrl-p` repeat it; `n` keeps its create action.
- Tree searches do not affect buffer searches.

**Quitting from the tree.** While the tree is focused, `:q`, `:wc`, and
`:close` (and their long and `!` forms) hide it and return focus to the
previous pane, keeping buffers, panes, and terminal sessions. `:qa` quits the
workspace as usual.

#### Width

The preferred width is `editor.directory_tree_width`: 33 columns by default,
12 to 240 allowed. Change it in `Space o o` or the configuration file.

| To | Do |
| --- | --- |
| Resize for this workspace | Drag the tree's right border |
| Widen / narrow by 5 from the tree | `:resize-right + 5` / `:resize-right - 5` |
| Widen the adjacent pane by narrowing the tree | `:resize-left + 5` from that pane |

Other edges behave as for ordinary panes. The tree always leaves at least 24
columns for the pane layout.

#### Refreshing the tree

- Listings load in the background only as directories are expanded.
- Directory symlinks are shown as links and never expanded recursively.
- While the tree is shown, expanded directories are checked every two seconds.
  Entries created, renamed, moved, or deleted outside Runyte appear
  automatically.
- The selected path stays selected while it exists; otherwise the nearest
  visible ancestor is selected.
- Collapsed branches and a hidden sidebar are not scanned. Expanding a branch
  refreshes it, and showing the sidebar resumes checking.

### Files changed outside Runyte

Runyte watches the parent directory of every open file and the directory of
every open explorer, in both modes, even while a persistent TUI is detached.

**When a path no longer agrees with the state accepted at open, save,
reload, or refresh:**

- Every pane showing it, and the status line, show `[STALE]`. The buffer list
  marks hidden stale buffers too.
- `[STALE]` is separate from `[+]`, so an edit that conflicts with disk reads
  `[+] [STALE]`.
- Text, selections, undo, and language-server state are kept.
- The first observation of each disk revision creates one WARNING
  notification.

When a stale file or explorer is attached to no pane, bringing it back into a
pane automatically refreshes it from disk if it has no unsaved edits. A buffer
shown in any pane keeps the behavior above, including when another pane is
maximized. A terminal covering a buffer puts that buffer in the background.
Deleted, unreadable, or binary replacements leave the retained contents and
stale marker intact.

**What you can do:**

| Command | Effect |
| --- | --- |
| `Space b d` or `:diff-disk` | Open the disk version as `[disk] path [RO]` beside your buffer, in a live side-by-side comparison |
| `Space r` or `:reload` | Reload. A clean buffer reloads at once; a dirty one asks first |
| `:write!` | Save over the changed file |

- **`:diff-disk`** reads the path again. The disk version is on the left and
  your editable buffer on the right; the comparison follows later edits.
  Deleted, unreadable, and binary versions, and versions over the comparison
  size limit, are refused without changing panes.
- **`:reload` on a dirty buffer** names the path and the undo history that will
  be discarded. Escape or `Ctrl-c` keeps the buffer. Enter applies only the
  exact disk revision reviewed; if the file changes again, the buffer is kept
  and the new revision must be reviewed.
- **`:write!`** is the explicit way to replace a changed file. An ordinary save
  refuses a known conflicting revision before save hooks can edit the buffer.
  `[STALE]` clears only after the written file is verified.

**Resolving by itself:**

- When your text and the disk text become equal, Runyte adopts the disk
  version and marks the buffer clean, keeping usable undo history.
- A deleted file stays stale, but an ordinary save may recreate it.
- Binary and unreadable replacements are never loaded into a text buffer.

### Comparing two files

| Command | Effect |
| --- | --- |
| `:diff-this` (`:difft`, `:dt`) | Mark this buffer, or compare it with the one marked before |
| `:diff-this` in the marked buffer | Take the mark back |
| `:diff-off` (`:do`) | Close the comparison |
| `:diff-disk`, `Space b d` | Compare with a fresh disk snapshot |
| `:diff-remote` | Compare a provider document with a fresh remote snapshot |
| `Space g D` | Compare the active file's Git versions; see [Git](git.md#git) |

**Example:**

1. Open `old.rs` and run `:dt`.
2. Open `new.rs` and run `:dt` again.

`old.rs` appears on the left and `new.rs` on the right. If `old.rs` was not on
screen, the second command splits for it. If both were already in panes, those
panes are used.

**Alignment:**

- Corresponding lines sit level. Where one side has lines the other lacks, the
  other side shows a hatched filler row. It belongs to no line: it cannot be
  clicked, labelled by `goto-word`, or moved onto, like the blank area past
  the last line.
- The two sides scroll together. The pane you are in leads and the other
  follows the line facing it, which is not the same line number once the
  files have drifted apart.

**Live changes.** Both files stay editable, and the comparison follows what you
type.

| Shown as | Meaning |
| --- | --- |
| Added | Lines only the right side has |
| Removed | Lines only the left side has |
| Changed | Lines that answer to different lines on the other side |

- Lines replaced by an unequal number of lines are one change, not a deletion
  plus an addition — the same folding the Git gutter does.
- The gutter shows the comparison instead of Git marks while the comparison is
  open.
- Large buffers are compared in the background after an edit. Until it
  catches up, change colours pause and the panes keep the known alignment,
  adjusting for inserted or removed rows. A whole-buffer replacement without
  an edit transaction may align rows by number meanwhile.

**Display:**

- Soft wrap is off while a comparison is open, whatever `editor.soft_wrap`
  says, and returns afterwards: a wrapped line would take different numbers of
  rows on each side.
- Folded regions are expanded for the same reason.

**Ending it.** A comparison needs both panes and both buffers. Closing either
pane, or showing a different buffer in one, ends it. The Git comparison from
`Space g D` also owns the split it created, so closing either side or
`:diff-off` collapses it and restores the previous buffer or the explorer.

### Shell pipes

`:pipe <shell-command>` (or `:| <shell-command>`) replaces each selection with
the command's output.

On Unix:

```text
:pipe sort
:pipe tr a-z A-Z
:pipe jq .
```

| On | Runs | Syntax |
| --- | --- | --- |
| Unix | `/bin/sh -c`, one invocation per selection | Shell syntax: arguments, quotes, and pipelines |
| Windows | Windows PowerShell with no profile, noninteractive, whatever `COMSPEC` says | PowerShell syntax |

**How it works:**

- Each selection is sent on stdin and replaced with stdout. Selected text is
  never inserted into the command, and editor variables are not expanded.
- Programs resolve through the inherited `PATH`.
- The working directory is the workspace root when the command starts.
- Stdout must be UTF-8 and is kept exactly, including trailing newlines. Empty
  stdout deletes the selection.
- All replacements are one undo step. Any failed invocation discards every
  replacement and reports an error.
- Commands can have side effects that undo and cancellation cannot reverse.

**On Windows:**

```text
:pipe [Console]::Write([Console]::In.ReadToEnd().ToUpperInvariant())
```

- That example uppercases each selection.
- Console input, output, and native pipeline output are UTF-8 without a BOM.
- Use `[Console]::In.ReadToEnd()` and `[Console]::Write(...)` when exact
  newlines matter; PowerShell object pipelines format text their own way.
- The workspace directory needs an ordinary Windows spelling shorter than 260
  UTF-16 units.

**Limits:**

| Limit | Value |
| --- | --- |
| Pipe jobs per workspace | 1 |
| Selections per job | 256 |
| Command text | 16 KiB |
| Selected input | 8 MiB |
| Combined stdout | 8 MiB |
| Retained stderr | 16 KiB per invocation |
| Whole-job deadline | 30 seconds |

Selections run one after another.

**While it runs:**

- Editing and rendering continue.
- The result goes to the buffer it came from, even after switching panes. It
  is refused if that buffer changed (including an edit then undo), closed, or
  became read-only. Moving selections does not retarget it.
- `:pipe-cancel` cancels the job.
- The workspace host owns the job in both modes, so detaching and reattaching
  does not restart or cancel it. Host shutdown cancels it.

**Cleanup.** Cancellation, timeout, failure, and completion kill the owned
process tree.

- Unix uses a process group and reaps the shell. Processes that deliberately
  leave the group are outside its cleanup.
- Windows uses a job that forbids breakaway. Inherited output pipes cannot keep
  a finished shell's job alive indefinitely.

There is no default key; the bare `|` key is reserved.

### System clipboard

`Space c y`, `Space c p`, `Space c P`, `Ctrl-v`, and `Alt-v` use the system
clipboard through:

| Platform | Helper |
| --- | --- |
| macOS | `pbcopy` / `pbpaste` |
| Windows | Native Unicode APIs |
| Linux and other Unix | The first available of `wl-clipboard`, `xclip`, or `xsel` |

A missing helper gives an actionable status message and does not affect
Runyte's own registers.

<!-- navigation -->
[Contents](README.md) · [Previous: Key bindings](key-bindings.md) · [Next: Panes](panes.md)
