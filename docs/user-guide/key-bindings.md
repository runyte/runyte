## Key bindings

The spellings in this guide are Runyte's defaults. The `keys` configuration
section can move the application and window prefixes, their descendants, and
advertised aliases; see [Custom key bindings](custom-key-bindings.md#custom-key-bindings). Live help,
hints, About, tutorials, and action messages always show the effective
spelling.

Other sections document the keys of their own views: [Files and
buffers](files-and-buffers.md#files-and-buffers), [Panes](panes.md#panes), [Terminals](terminals.md#terminals),
[Git](git.md#git), [Language support](language-support.md#language-support), and [Workspaces and
persistent sessions](workspaces-and-persistent-sessions.md#workspaces-and-persistent-sessions).

### Editing

The one-key rows in this table are the complete direct-binding inventory for
Normal and Select modes. Common prefixed gestures follow for context. Explorer
keys are under [Directory buffers](files-and-buffers.md#directory-buffers), and Insert-mode keys
under [Insert and Replace modes](key-bindings.md#insert-and-replace-modes).

| Key | Action |
| --- | --- |
| `Shift-Left` / `Shift-Right` | Visit the previous / next running persistent session (mux mode) |
| `h` / `j` / `k` / `l`; `Left` / `Down` / `Up` / `Right` | Move left, down, up, right |
| `w` / `b` / `e` | Next word / previous word / word end |
| `W` / `B` / `E` | Long-word variants |
| `f` | Find the next typed character in the buffer |
| `F` / `t` / `T` | Find backward, or move until a character forward / backward |
| `Home` / `0`; `^`; `End` / `$` | Start / first non-whitespace / end of line |
| `Ctrl-b` / `Ctrl-f`; `Ctrl-u` / `Ctrl-d` | Page up / down; half-page up / down |
| `PageUp` / `PageDown` | Page up / down |
| `gg` / `ge` or `G` | Start / end of file |
| `gf` | Open the selected path exactly, or infer the complete path under a bare cursor; inferred paths at the end of a sentence ignore trailing punctuation if the literal name does not exist. On a Markdown link to `#heading` or `file.md#heading`, land on that heading |
| `gw` | Dim the view, label nearby words with one key and farther words with two, then type a label to jump |
| `gt` / `gc` / `gb`; `H` / `M` / `L` | Move to the top / center / bottom of the visible window |
| `i` / `a` / `I` / `A` | Insert before/after cursor or at line boundary |
| `o` / `O` | Open line below / above |
| `r` / `R` / `~` | Replace once / enter Replace mode / toggle case |
| `v` | Enter Select mode |
| `x` / `X` | Grow whole lines below / above, never shrinking |
| `%` | Select the entire buffer |
| `C` / `Alt-C` | Add a cursor on the nearest line below / above holding a character at the cursor's column, skipping the ones too short |
| `V` / `Alt-V` | Add a cursor on the next / previous line, padding short or empty lines with spaces to the same display column |
| `;` / `Alt-;` | Collapse selections / flip their direction |
| `,` / `Space s c` / `Alt-,` | Keep only the primary selection / drop it |
| `)` / `(` | Make the next / previous selection primary |
| `Alt-)` / `Alt-(` | Rotate selection contents forward / backward |
| `&` | Pad until every cursor shares the rightmost display column |
| `_` | Delete trailing whitespace from every selected line; `%` then `_` strips the buffer |
| `Alt-_` | Shrink every selection past the whitespace at its ends, without changing the text |
| `Space p .` | Toggle dim `·`, `→`, and `¬` markers for spaces, tabs, and line endings |
| `d` / `c` | Delete / change selection or cursor character; `d` after transient `x`/`X` cuts whole lines |
| `y` / `p` / `P` | Yank selection or cursor character, leaving a caret / replace the selection, or paste after a bare caret / paste before |
| `Y` | Yank every line the selection touches, as whole lines, leaving a caret |
| `>` / `<` | Indent / unindent |
| `J` | Join the selected lines with a space, or pull the line below up to a single-line selection or caret |
| `Ctrl-c` | Comment or uncomment every line the selection touches, using the buffer language's line comment; also bound in Insert mode |
| `u` / `U` | Undo / redo |
| `Alt-u` / `Alt-U`; `Space s u` / `Space s U` | Undo / redo selection changes, back to the last text edit |
| `s` / `/` | Search with an escaped literal, ignoring case / with a regular expression |
| `n` / `N` | Step to the next / previous match |
| `*` | Select every occurrence of the word or selection under the caret |
| `Ctrl-o` / `Ctrl-i`; `Alt-o` / `Alt-i` | Jump backward / forward through every navigation point; jump backward / forward to another buffer |
| `Tab` | Open contextual actions for the selection or row under the caret |
| `Ctrl-s` | Save |
| `Ctrl-C` (Ctrl-Shift-c) | Copy the selection to the system clipboard |
| `Cmd-c` (macOS) | Copy the selection to the system clipboard |
| `Ctrl-v` / `Alt-v` | Paste the system clipboard, storing an image in the workspace and writing a numbered Markdown link to it; also bound in Insert mode |
| `:` | Open the command palette |
| `\|` | Shell pipe key (reserved; use `:pipe <shell-command>`) |
| `<n>` before a command | Repeat a motion or countable command |
| `<n>gg` / `<n>G` | Go to line `<n>` |
| `"` then a register | Select a named register; uppercase appends and `_` discards |
| `Space m …` | Record, replay, and list macros; see [Macros](key-bindings.md#macros) |
| `mm` | Jump to the matching bracket |
| `m i …` / `m a …` | Select inside / around a word, paragraph, delimiter pair, function, type, or argument; see [Text objects](key-bindings.md#text-objects) |
| `m s …` / `m r … …` / `m d …` | Add / replace / delete the pair around each selection; see [Surround editing](key-bindings.md#surround-editing) |
| `z…` / `Z…` | View alignment and scrolling |
| `Esc` / `Ctrl-\` (`Ctrl-4` on legacy terminals) | Return to Normal mode |

#### Selecting whole lines

| Key | Command | Effect |
| --- | --- | --- |
| `x` | `select-line` | Snap to whole lines, then grow downward without dropping lines already selected |
| `X` | `extend-line-above` | Expand a partial selection to whole lines, then grow upward without dropping lines already selected |
| — | `extend-line-below` | Same behavior as `select-line`, for custom bindings |
| — | `select-line-up` | The former upward edge-walking behavior, for custom bindings |

For example, `x x X` adds the line above the two selected lines. All four
support counts and multiple selections.

#### Selection history

`Alt-u` (`selection-undo`) brings back a selection lost to a motion, Escape,
a collapse, or removing extra cursors. `Alt-U` (`Alt-Shift-u`,
`selection-redo`) reapplies it. `Space s u` and `Space s U` are the same
commands. Like the other selection commands, they are keys rather than `:`
commands.

- Every range, its direction, the primary range, and the selection mode come
  back, including `x`/`X` whole-line behavior. Text never changes.
- A counted motion, an accepted search, or a mouse drag is one step. Cancelled
  search previews add none. A new selection change clears redo.
- History is per pane and per buffer, works in read-only buffers, and survives
  switching buffers.
- Any text change, including undo/redo or an edit in another pane, starts a
  new history.
- Restoring never enters Insert/Replace mode or reopens a prompt.
- Limits: 128 states and 4,096 total ranges per history, for up to 32 buffers
  per pane. An oversized selection ends that history.
- Terminal review is not supported.

#### Selecting motions

In Normal mode, `w`, `b`, `e`, `W`, `B`, `E`, `f`, `t`, `F`, and `T` select the
text they cross, as in Helix:

| Keys | Result |
| --- | --- |
| `w d` | Delete a word and the space after it |
| `e c` | Change to the end of the word |
| `w p` | Replace what `w` selected — the word and the space after it — with the register |

- `w` selects through the whitespace after the word, `e` through the end of the
  next word, and `b` back to the start of the previous one.
- `f` selects from the caret through the found character, `t` up to it.
- Each press starts a fresh selection, which never contains a line break.
- Anything that reads a selection reads this one: `p` replaces it, `s` and `/`
  search inside it, and `*` searches for it.
- Other motions still move a caret, and Select mode still extends. Its word
  motions extend to where these selections end, so `v w` takes `one ` from
  `one two`, as in Helix, whether or not `editor.selecting_motions` is on.

Set `editor.selecting_motions: false` to make these motions move a caret too.
A word is then selected with `v` first: `v e d` deletes to the end of the word.

#### Yank and paste

**Yank ends the gesture.** `y` and `Y` return to Normal mode and collapse each
range to a caret on the last character copied, one caret per selection.

- That makes `y` then `p` a duplicate: the caret pastes past the text just
  yanked. A range that outlived the yank would be invisible in Normal mode
  and would make `p` replace the yanked text with itself.
- `Space c y` is the exception. It is also what a right click does, so it
  copies without disturbing the selection.

**`p` depends on the selection**, as `d` and `c` do:

| Selection | `p` does |
| --- | --- |
| A bare caret | Paste after the caret |
| A range holding text | Replace that text |
| A bare caret, linewise register (from `x y` or `Y`) | Paste whole lines |
| A characterwise selection, linewise register | Replace exactly the span, with the register's final line ending removed; inner line breaks stay |
| An `x`/`X` or Vim line selection, linewise register | Replace whole lines |

- The register is not used up and the replacement stays selected, so the same
  content can be pasted over one range after another.
- A multi-selection from a search replaces every match at once.
- `P` never replaces. It is the way to paste at the start of a selection
  without giving it up.
- `Space c p` and `Space c P` follow the same rules from the system clipboard.

#### Pasting images

`Ctrl-v` and `Alt-v` paste from the system clipboard in Normal, Select,
Insert, and Replace modes. When the clipboard holds an image:

1. The image is stored under `.runyte/cache/images/` in the workspace, named
   by the hash of its content.
2. The document gets a numbered Markdown link, such as
   `[Image 1](.runyte/cache/images/1f0a2b3c4d5e6f70.png)`.

- Pasting the same screenshot twice stores one file.
- The number continues past the highest `[Image N]` already in the document,
  rather than counting links.
- `?` renders the link as **Image 1**, without the path.
- A path with a space or a parenthesis is written in angle brackets,
  `[Image 1](<My Notes/a (copy).png>)`, the only spelling that survives being
  read back.

**When text is pasted instead.** A clipboard with no image, a machine with no
helper that can supply one, or an unresponsive display server all paste text,
exactly like `Space c p`.

A clipboard offering text *as well as* a picture counts as text. Copying
spreadsheet cells or formatted text attaches a rendered bitmap for "paste as
picture" commands, and taking it would lose the text you selected. A copied
picture has no text beside it, which is what screenshot tools and a browser's
"Copy Image" produce. If `Ctrl-v` pastes text where you expected an image, the
source is offering text. Once the clipboard has said it holds an image, failing
to fetch it is reported rather than silently pasting text.

Both keys are unbound in Terminal Insert, where they reach the program if the
outer terminal passes them through.

**The cache is scratch.** `.runyte/` is not tracked by Git, so a pasted image
travels with the working copy, not the commit.

- A document read from another clone needs its images moved somewhere the
  repository carries, and its links updated.
- Nothing promises to keep a file there. A document meant to outlive the
  session should not point into it.

**Storage safety:**

- Directories and images are private: `0700` and `0600` on Linux and macOS,
  owner-only permissions on Windows local NTFS.
- Symlinked or reparse-point storage paths, hard-linked entries, and files
  whose bytes disagree with their content-hash name are refused.
- Writes use exclusively created temporary files and atomic publication.

**On Windows:**

- Registered PNG data is read directly; supported native bitmap data is
  converted to PNG. Plain text still takes precedence.
- Clipboard data and converted images are limited to 64 MiB, and bitmap
  dimensions must fit in 64 MiB of RGBA pixels.
- Unsupported compressed bitmap layouts and custom colour profiles report an
  error.
- The editor waits at most one second for the clipboard. If Windows takes
  longer, another request reports the clipboard as busy; retry after the
  worker finishes.

<a id="insert-mode"></a>

### Insert and Replace modes

| Mode | Entered with | Typing |
| --- | --- | --- |
| Insert | `i`, `a`, `I`, `A`, `o`, `O`, `c` | Adds text at every caret |
| Replace | `R` | Overwrites the character ahead of each caret |

Escape returns to Normal mode. After `a` the caret rests on the last character
typed, and after `i`, `I`, or `c` on the character the text went in front of,
as in Helix. After `A`, `o`, or `O` it rests on the last character typed, where
Helix rests on the line terminator a Runyte caret never occupies. Moving the
caret in Insert mode, by key or pointer, leaves it where it was put.

**Replace mode details:**

- Entering collapses every selection to its active head.
- A caret at line end appends instead of overwriting.
- A newline inserts a line break rather than consuming the existing one.
- Characters, including whole emoji sequences, are replaced one for one, and
  CRLF stays one line ending.
- The whole Replace session is one undo checkpoint.
- Lowercase `r` is the single-character Normal-mode command and never enters
  Replace mode.

The keys shared by Insert and Replace modes:

| Key | Action |
| --- | --- |
| `Shift-Left` / `Shift-Right` | Visit the previous / next running persistent session (mux mode) |
| `Esc` / `Ctrl-\` (`Ctrl-4` on legacy terminals) | Return to Normal mode |
| `Backspace` / `Shift-Backspace`; `Delete` | Delete to the previous indentation stop in leading whitespace, otherwise the previous character / delete the next character |
| `Alt-Backspace` / `Alt-Delete` | Delete the previous / next word |
| `Ctrl-u` / `Ctrl-k` | Delete to the start / end of the line |
| `Enter` / `Ctrl-j` | Insert a newline with the current indentation and optional smart indentation |
| `Tab` / `Shift-Tab` | Insert the configured indent style / the other style (`spaces` to the next tab stop, or one tab) |
| `Left` / `Down` / `Up` / `Right` | Move the caret |
| `Home` / `End`; `PageUp` / `PageDown` | Move to a line boundary; move by a page |
| `Ctrl-x` | Ask the language server for completions |
| `Ctrl-c` | Comment or uncomment the lines holding the carets |
| `Ctrl-s` | Save |
| `Ctrl-C` (Ctrl-Shift-c) | Copy the selection to the system clipboard |
| `Cmd-c` (macOS) | Copy the selection to the system clipboard |
| `Ctrl-v` / `Alt-v` | Paste the system clipboard, storing an image in the workspace and writing a numbered Markdown link to it |
| `Ctrl-w` then a pane suffix | Move to another pane without first leaving Insert or Replace mode |

In Insert mode, `Alt-Backspace` (Option-Backspace on macOS) removes all leading
indentation before the caret in one press and stops at the start of the current
line. Pressing it at column zero can delete across the preceding line break.

**Deleting in Replace mode.** Backspace or Shift-Backspace retraces the current
overwrite run: overwritten characters come back, and characters appended past
line end are removed. Alt-Backspace and `Ctrl-u` restore by word and to the
start of the line.

**Shift-Backspace** in Insert mode deletes like Backspace, so Shift can stay
held while correcting uppercase text.

**Leaving.** Escape or `Ctrl-\` returns to Normal mode.

#### Commenting lines

`Ctrl-c` comments or uncomments every line the selection touches, using the
buffer language's line comment.

| Marker | Languages |
| --- | --- |
| `//` | Rust, C, C++, C#, Go, Java, JavaScript, TypeScript, TSX, Kotlin, Swift, Zig, Protobuf, PHP, JSONC |
| `#` | Python, Bash, TOML, YAML, CMake, Make, Dockerfile, HCL, Ruby, Elixir |
| `--` | SQL, Lua |
| `;` | INI |
| none | CSS, HTML, JSON, XML, Markdown — the key reports this and changes nothing |

- The marker goes at the least-indented line, so nested lines keep their
  relative indentation.
- Blank lines are left alone both ways.
- Uncommenting removes the marker and at most one space after it: `// x` and
  `//x` both become `x`.
- A partly commented block becomes fully commented first, so a second press
  always undoes the first.
- In an extensionless script whose language comes only from its shebang, the
  shebang line is left unchanged, so the buffer keeps its language.
- In Insert mode `Ctrl-c` acts on each caret's own line. Entering Insert mode
  collapses selections to carets, so choose a block in Normal or Select mode.

#### Auto-closing pairs

`editor.auto_close` is off by default. Turn it on in `Space o o` to pair typed
`()`, `[]`, `{}`, single quotes, and double quotes in Insert mode.

- The caret stays inside the pair. Typing the closer already under the caret
  moves past it.
- Backspace between an empty pair removes both characters, including pairs
  already in the document.
- Openers pair before whitespace, closing punctuation, or the end of the
  buffer.
- Quotes also need a non-word character before them. Escaped quotes and
  apostrophes inside words stay literal.
- It is a local text heuristic, independent of language syntax.
- Paste, Replace mode, prompts, and terminal input stay literal.
- Each caret is handled independently.

#### New lines and indentation

In Insert mode, Tab normally advances to the next visual tab stop. Backspace within
leading spaces and tabs removes indentation to the previous stop. With width 4,
Backspace moves columns 8 → 4, 6 → 4, and 4 → 0 (columns counted from zero).
Mixed spaces and tabs use their displayed columns. After text, Backspace deletes
one character; at the start of a line it keeps ordinary newline joining.
Selections, paired characters, and Markdown list handling retain their existing
behavior. Replace-mode Backspace still restores overwritten text.

`editor.tab_width` and `editor.indent` supply defaults. Language and file-pattern
exceptions apply to individual buffers, including their display, wrapping,
mouse positioning and language-server formatting options. See
[Indentation overrides](configuration.md#indentation-overrides).

| Key | Keeps | Adds |
| --- | --- | --- |
| `o` / `O` (Normal) | The row's exact leading spaces and tabs | Nothing: no list marker, no syntax level |
| Enter (Insert) | The row's exact leading spaces and tabs | At most one level in `editor.indent` style when the row opens a block the syntax indentation query recognizes: it ends with an opening bracket in a brace language, a Python header or YAML key ends with `:`, or a `case` label, an element's start tag, or a construct such as `else:` begins on it. An ordinary row inside a block adds nothing. A grammar that requires a tab still gets a tab |

`o` and `O` place the caret after the indentation, and work the same with
smart newline off.

**Markdown lists** continue on Enter while `editor.smart_newline` is on (the
default):

- bullets keep their marker, numbered and lettered items advance, and task
  items start unchecked;
- Enter on an empty item ends the list;
- Tab on a whitespace-only line immediately below a list aligns with the item's
  content, including through continuation lines and nested lists. It fills the
  remaining distance with spaces even when the configured indent style is tabs.
  Once aligned, further Tab presses use the configured style and tab stops.
  This also applies to scratchpads with `editor.scratch_markdown` enabled;
  Shift-Tab keeps its ordinary other-style behavior;
- Backspace right after a marker turns it into a continuation indent, whether
  or not text follows; another Backspace removes that alignment in one press;
- Enter and Backspace renumber the following numbered or lettered items that
  were in sequence, skipping nested items, continuation lines, and blank
  lines. A list numbered `1.` throughout, or one with a gap, keeps its numbers
  from that point on;
- a single `I.` or `V.` advances as a letter unless the previous sibling
  establishes Roman numbering.

In other file types, smart newline keeps the alignment under a list item's
content. With `editor.smart_newline: false`, Enter keeps only the row's
leading indentation and Tab uses ordinary indentation stops. List alignment
looks back through at most 256 rows of at most 4096 characters each, stopping
at a blank line or unrelated text; beyond those bounds Tab uses ordinary
indentation.

Unsupported, malformed, oversized, and unterminated-final-line cases keep the
exact prefix and never block the newline.

### Search

Search has two flavours and one behavior:

| Key | Flavour |
| --- | --- |
| `s` | Literal text, ignoring case. The pattern is escaped, so `foo(` and `a.b` find themselves. No wildcards. |
| `/` | A regular expression, matching case unless the pattern says otherwise. Use it for wildcards or case-sensitive search. |

There is no separate case-sensitive literal key: use `/`, escaping any regex
syntax, when case matters, and `(?i)` in a `/` pattern to ignore case.

#### How a search selects

Every search selects *all* its matches at once, so the next edit applies to
all of them.

- Each match is selected in full, with the cursor on its last character, where
  an append or a motion continues from.
- The primary match has the primary-selection ground and one cursor in the
  Select colour (pink in `ember-dark` and `ember-light`, orange in most other
  built-in themes). Other matches keep the secondary selection colour without
  cursors.
- The status line shows the primary's position among the results.

**Then:**

| Next action | Effect |
| --- | --- |
| An edit straight away | Changes every match |
| `n` / `N` | Selects only the next / previous match; further presses cycle that single selection through the remembered results |
| An edit after `n` / `N` | Changes only the match you reached |
| A selection motion such as `e` | Turns the results into ordinary selections, with their Select-coloured endpoint cursors, and shows the selection count |

#### Preview while typing

- Every match the pattern would select is highlighted as Enter would leave it.
- The pane scrolls to the match that would become primary: the first at or
  after the cursor.
- Nothing is selected until Enter. Escape returns the pane to where it was.
- A pattern matching nothing highlights nothing and returns the view.
- An unfinished regular expression such as `foo(` keeps the last preview until
  it is valid again.
- The rest of the pane stays dimmed under the prompt.

#### Searching inside a selection

When at least two characters are selected — from `v` and a motion, `x`, `%`,
or a previous search — the search looks only inside that text, and `n`/`N`
wrap within it.

Searches therefore narrow step by step:

1. `x x x` selects three lines.
2. `s` finds the calls in them.
3. `/` picks out one argument.

Press `;` to collapse to a caret and search the whole buffer again.

#### Regular expressions

`/` passes the pattern directly to Rust's `regex` engine. The `/` key opens
the prompt; it is not a delimiter in a JavaScript-style `/pattern/flags`.

| Want | Write | Not |
| --- | --- | --- |
| Case-insensitive | `(?i)hello` | `/hello/i` |
| Case-sensitive | `Hello` (the default) | |
| Scoped flag | `(?i:hello) World` | |
| Across lines | `(?s)foo.*bar` or `foo\nbar` | |

| Inline flag | Meaning |
| --- | --- |
| `i` | Case-insensitive |
| `m` | `^` and `$` match at line boundaries |
| `s` | `.` also matches newline |
| `R` | CRLF-aware multiline boundaries |
| `U` | Swap greedy and lazy repetition |
| `u` | Unicode (on by default) |
| `x` | Verbose mode |

- **Supported:** character classes, alternation, groups, greedy and lazy
  repetition, anchors, word boundaries, Unicode properties, `\d`, `\s`, `\w`.
- **Not supported:** slash-delimited expressions, trailing flags,
  look-around, backreferences.
- Capturing groups compile, but only the complete match is selected.
- Buffer search runs over the whole text, so a pattern can span lines.
  Workspace regex search is line-scoped, because each result names one row, so
  `Space / /` cannot return a multiline match.

`:help regex` has more examples and a compact syntax table.

#### Search keys

`s` and `/` search this buffer. `Space /` widens the same two to the whole
project: `Space / s` mirrors `s` and `Space / /` mirrors `/`. The Finder is
`f` rather than a sigil — `Space f`, its namespace spelling `Space / f`, and
`Space g f` over commits — and `Space / a` and `Space / p` open that Finder
over a wider scope. `Space s` holds selection commands only.

| Key | Action |
| --- | --- |
| `s` | Search the buffer, ignoring case |
| `/` | Search the buffer with a regular expression |
| `n` / `N` | Select only the next / previous match |
| `*` | Select every occurrence of the word or selection under the caret |
| `f` | Find the next typed character in the buffer |
| `Space / s` | Search the workspace, ignoring case |
| `Space / /` | Search the workspace with a regular expression |
| `Space f` or `Space / f` | Open the Finder over files, buffers, and terminals by name or content; `Tab` switches modes and `Ctrl-t` toggles preview |
| `Space / a` | The same Finder over every file, ignore files not consulted |
| `Space / p` | The same unfiltered Finder rooted at a typed path, inside the workspace or outside it |
| `Tab f` in a directory | The same Finder rooted at the directory the explorer shows (`:open-explorer-finder`) |
| `Space s c` | Keep only the primary selection in any multi-selection |
| `Space s e` / `Space s b` | Put a cursor at the end / start of every selected line |
| `Space s a` or `&` | Pad with spaces until every cursor shares the rightmost display column |
| `Space s k` / `Space s r` | Keep / remove selections matching a typed regular expression |

The directory-scoped pickers have no key: `:file-picker-directory` and
`:fuzzy-grep-directory` search below the active file or explorer directory.
The Finder is described under [The Finder](files-and-buffers.md#the-finder).

#### Workspace search

`Space / s` and `Space / /` search every file in the project.

**What is searched:**

- No ignore file is consulted: a gitignored path the Finder omits is still
  searched.
- `.git`, `.runyte`, and `target` are skipped by name, and symlinks are
  skipped.
- `editor.show_hidden_files` applies.
- Files larger than 4 MiB are not read.

**While it runs:**

- Accepting the prompt queues the walk in the background and returns to
  input. The result buffer opens when it completes.
- A newer workspace search supersedes an older one, so a late result cannot
  replace a newer query.
- The status row is temporarily replaced by `Searching workspace`, the query,
  the elapsed time, and a rotating `- \ | /` spinner.

**The result buffer** is one read-only `[workspace search]` buffer, replaced by
each new search.

- Rows are `path:line:column`, a snapshot taken at query time.
- Enter opens the result under the cursor. Movement, selection, copying, `s`,
  `/`, splits, help, buffer switching, and jump history work as usual.
- Visit as many results as you like without rerunning the query; rerun it to
  refresh.

This differs from the Finder's content mode, whose fuzzy query stays live and
so remains a choose-one picker.

### Layout and whitespace display

`Space p` holds text presentation and the commands that lay out selected
lines.

| Key | Action |
| --- | --- |
| `Space p .` | Toggle whitespace markers for this session |
| `Space p s` | Toggle soft wrap for this session |
| `Space p w` | Hard-wrap each selection at `editor.hard_wrap_width` |
| `Space p r` | Reflow selected prose at `editor.hard_wrap_width` |
| `Space p j` | Join selected lines with a typed delimiter |
| `J` | Join selected lines with a space |
| `Space p t` | Align the columns of the selected table |

After an edit that inserts or removes line breaks—such as Enter, hard wrap,
reflow, opening a line, multiline paste, or joining lines—the editing pane
returns to column zero when the caret fits. Otherwise it scrolls right only
far enough to keep the caret visible, accounting for tabs, wide characters,
and the space occupied by the gutter. Rejected edits and unchanged wraps keep
the view. Changing soft wrap through `Space p s` or configuration also resets
each affected pane; diff panes remain unwrapped. The caret keeps its editing
position. Exceptionally long runs of invisible characters use a bounded view
near the caret to keep scrolling responsive.

#### Whitespace markers

`Space p .` shows:

| Character | Marker |
| --- | --- |
| Space | `·` |
| Tab | `→`, followed by enough cells to reach the same tab stop |
| LF or CRLF line ending | one `¬` |

An unterminated final line has no marker. The markers are display-only: text,
offsets, selections, wrapping, and saved files are unchanged. The setting is
`editor.render_whitespace`.

#### Soft wrap

`Space p s` toggles `editor.soft_wrap` for this session.
When enabled, the global status line shows `↪` before the cursor position.
The arrow reports this general setting, even when a rendered Markdown page
wraps independently. `editor.always_wrap_rendered_markdown` defaults to `true`;
set it to `false` to make rendered pages follow `Space p s` too.

- Wrapping follows the live pane width, including after a resize. It does not
  use the hard-wrap width and never changes the text.
- It breaks at word boundaries, and within a word only when the word is wider
  than the pane.
- A document whose longest line exceeds 64,000,000 bytes is never wrapped. The
  length is measured once, when the file is read, before anything is drawn.
- Layouts are reused while the text, pane width, and tab width stay the same,
  so moving through a minified file of a few megabytes does not recalculate
  its whole line each keypress. Drawing seeks straight to the visible text,
  even near the end of the line. An edit or a new width recalculates the
  affected layout; the line-length limit above protects against extremely
  costly layouts.

#### Hard wrap and reflow

| | `Space p w` (hard wrap) | `Space p r` (reflow) |
| --- | --- | --- |
| Width | `editor.hard_wrap_width` (80) | `editor.hard_wrap_width` (80) |
| Existing newlines | Kept as boundaries | Refilled; blank lines stay paragraph boundaries |
| Long words | Kept whole unless one word is wider than the line | Kept whole |

**Reflow in Markdown** keeps headings, fenced and indented code, block quotes,
tables, thematic breaks, and separate bullet or numbered items. Wrapped list
continuations get a hanging indent.

**Reflow in source files** changes only `#` and `//` line-comment paragraphs,
repeating the indentation and comment leader on every line, so selecting
nearby code can never join statements. Lists inside comments get the same
hanging indent.

**The scratch buffer** has no path to name a language, so
`editor.scratch_markdown` (on by default) makes `Space p r` refill it as
Markdown.

- Set it to `false` to reflow a scratchpad as plain text.
- Reflow and `?` read the option together: a scratchpad whose lists survive a
  refill is also one that renders. Highlighting, completion, and saving are
  unaffected.
- Scratch text whose first lines already identify a language keeps it, and is
  refilled as a source file.

#### Joining lines

`Space p j` is the inverse of `Space p w`: it removes every line break inside
the selection and asks what to put in their place.

| Typed | Result |
| --- | --- |
| `Space p j Enter` | Lines run together |
| `Space p j Space Enter` | Lines separated by a space |
| `Space p j`, any other text, Enter | That text, such as `, `, ` \| `, or a word, inserted literally |

- Select the lines first: `x` or `X` for whole lines, `v` for part of them.
- Whitespace against each removed break goes with it, so joining indented
  lines leaves no runs of spaces. The first line keeps its indentation.
- Only the selection is joined. A single-line selection reports that it holds
  no line break, and the line after the selection is never drawn in, even
  when a pointer drag ends on it.
- A selected blank line joins as an empty piece and leaves its own delimiter.
- All selections are joined in one transaction, so one undo reverts them.

`J` is the promptless join from Vim and Helix:

- It joins the lines a selection touches with one space, like
  `Space p j Space Enter`.
- A single-line selection or a bare caret pulls the line below up instead.
- A blank line contributes neither text nor a space, so `J` above an empty
  line removes it without a trailing space.
- The file's final line terminator is never joined away, even after `%`.

#### Formatting tables

`Space p t` pads every cell to the widest one in its column:

```
| Column 1 | Column 2 |
|---|---|
| Value | abc |
| Longer text | Very very long text |
```

becomes

```
| Column 1    | Column 2            |
|-------------|---------------------|
| Value       | abc                 |
| Longer text | Very very long text |
```

**What counts as a table:** rows opening with `|`, cells divided by `|`, and at
least one separator row of dashes. Without a separator, a selection is refused
even if every line looks like a row. The separator may be anywhere, so a
selection may start on it or end below a footer rule.

**What is kept:**

- A `+---+---+` separator keeps its `+` signs.
- GitHub alignment colons `:---`, `:---:`, and `---:` survive and set left,
  centred, or right alignment.
- An escaped `\|` stays inside its cell.
- A tab inside a cell becomes spaces at `editor.tab_width` stops, because a
  column boundary cannot be worked out from a tab.

**Selecting:**

- Select the rows first. This command widens the selection to whole rows.
- Selections on the same or consecutive rows are formatted together as one
  table.
- Blank lines inside the selection are allowed and left alone, so selecting a
  little more than the table is fine. Rows below the selection are never
  drawn in.
- Rows with different cell counts are squared up with empty cells; no cell is
  dropped.
- All rows take the indentation of the first.
- If any line is neither blank nor a row, or there is no separator, nothing
  changes and an INFO notification says no table was detected.

### Text objects

`m i` selects inside a text object and `m a` around it, as in Helix. Every
object works at every cursor. The result is left in Select mode, so `d`, `c`,
`y`, and `p` act on it and a motion extends it.

| Key | Object |
| --- | --- |
| `m i w` / `m a w` | The word under the cursor / with the space beside it |
| `m i W` / `m a W` | The WORD, everything between whitespace / with the space beside it |
| `m i p` / `m a p` | The paragraph's lines / with the blank lines beside them |
| `m i (` / `m a (`, also `[`, `{`, `<`, `"`, `'`, `` ` `` | Inside / around the enclosing pair; closing brackets are aliases |
| `m i m` / `m a m` | Inside / around the closest enclosing pair of any of those kinds |
| `m i f` / `m a f` | Inside / around the enclosing function |
| `m i t` / `m a t` | Inside / around the enclosing type, such as a class or struct |
| `m i a` / `m a a` | Inside / around the enclosing argument or parameter |

**Examples:**

| Keys | Result |
| --- | --- |
| `m i w c` | Change the word under the cursor |
| `m a ( d` | Delete a parenthesised group with its parentheses |
| `m i p d` | Delete the paragraph's lines |
| `m i " y` | Yank the text inside the quotes |
| `m a f` | Select the enclosing function |

**Words and paragraphs** are read from the text alone, so they work in every
buffer.

- A word is a run of letters, digits, and `_`; a run of punctuation; or a run
  of whitespace. A WORD is everything between whitespace.
- Around adds the whitespace after the word, or before it when nothing follows
  on the line.
- Neither crosses a line break. A cursor on an empty line has no word.
- A paragraph is a run of lines holding text, separated by empty or
  whitespace-only lines. On a blank line, `m i p` selects the run of blank
  lines.
- Around adds the blank lines after the paragraph, or before it at the end of
  the file.
- A paragraph is selected as whole lines, like `x`, so `m i p d` removes its
  lines and `x` extends the selection.

**Delimiter pairs** resolve through the syntax tree when there is one, which
tells a bracket in code from one in a string. Otherwise a balanced text scan
answers: in plain text, in languages without a grammar, while a file is still
parsing, or when the tree finds no pair (as inside a comment).

- Brackets nest and may span lines. The scan reaches 65,536 characters on each
  side of the selection.
- Quotes pair up from the start of the line, so a quoted string spanning lines
  is not found by the scan.
- A backslash-escaped delimiter is text.
- Asking again with a pair selected grows to the next pair out.
- In ordinary Markdown prose, the scan is first bounded to the enclosing
  Markdown syntax node.

**Functions, types, and arguments** need the syntax tree. Without one they say
why and leave the selection alone.

### Surround editing

`m s`, `m r`, and `m d` add, replace, and delete the pair of quotes or
brackets around each selection, as in Helix. The keys after them name the
pairs.

| Keys | Action |
| --- | --- |
| `m s` then a character | Surround each selection with that pair |
| `m r` then two characters | Replace the surrounding pair named by the first with the pair named by the second |
| `m d` then a character | Delete the surrounding pair |

**Examples:**

| Text and cursor | Keys | Result |
| --- | --- | --- |
| `name` with `name` selected | `m s "` | `"name"`, selected with its quotes |
| `(a, b)` with the cursor on `b` | `m r ( [` | `[a, b]` |
| `say("hi")` with the cursor on `h` | `m d "` | `say(hi)` |
| `[x, {y}]` with the cursor on `y` | `m d m` | `[x, y]` |
| `word` under the cursor | `m i w m s *` | `*word*` |

- Either bracket names its pair: `(` and `)` both mean parentheses, as do
  `[`/`]`, `{`/`}`, and `<`/`>`. Any other character is used on both sides, so
  `m s *` and `m s |` work.
- `m r` and `m d` find the pair as `m a` does, from the character under each
  cursor: through the syntax tree first, then by a balanced scan of the text.
  They know the seven pairs `m a` knows, `(`, `[`, `{`, `<`, `"`, `'`, and
  `` ` ``, and `m` for the closest of any of them.
- With a pair selected, as `m a (` leaves it, `m d (` deletes that pair rather
  than the one outside it.
- `m s` selects the result, delimiters included. `m r` and `m d` keep the
  selection on the text it covered.
- If any cursor has no surrounding pair, nothing changes and the status line
  says so. Cursors inside the same pair edit it once.
- `Esc` cancels while the keys are pending.

### Structural syntax

| Key | Action |
| --- | --- |
| `Space x e` / `Space x s` | Expand / shrink syntax selection |
| `Space x p` / `Space x c` | Select syntax parent / first child |
| `Space x h` / `Space x l` | Select previous / next syntax sibling |
| `Space x o` | Open the immediate Tree-sitter document outline |
| `Space x x` | Toggle the syntax fold at the cursor |
| `Space x f` / `Space x u` | Fold / unfold all syntax regions in this pane |
| `Space x [ f/c/p` | Go to the previous function / class / parameter |
| `Space x ] f/c/p` | Go to the next function / class / parameter |

- **Bounds.** Expansion keeps Tree-sitter's half-open bounds. Relationship
  commands and the syntax [text objects](key-bindings.md#text-objects) show them with the
  block cursor on the last included character, as in Select mode. Yank,
  delete, change, and indent act on exactly the highlighted span.
- **Document outline.** Nested entries are indented two spaces per level,
  up to 32 spaces. Filtering preserves their indentation and parent breadcrumbs.
- **Folds are per pane.** Two panes may collapse different regions of one
  shared buffer. Any edit clears both panes' folds.
- **Folded rows** show an accent-coloured `▸` between the line number and the
  gutter rule, and a muted `… N lines` suffix counting the hidden rows.
- **While parsing.** `Space x` commands and `mm` are dimmed until the syntax
  tree is ready; see [Syntax highlighting](language-support.md#syntax-highlighting).

### Macros

Macros live in the `Space m` namespace.

| Key | Action |
| --- | --- |
| `Space m m` | Start recording the default macro, or stop the recording |
| `Space m M` then a register | Start recording a macro under that register |
| `Space m r` | Replay the default macro; accepts a count |
| `Space m R` then a register | Replay that macro; accepts a count |
| `Space m l` | List every recorded macro; Enter replays the selected one |

**Recording:**

- `Space m m` starts recording and the same keys stop it. The stopping keys
  are not recorded.
- `Space m m` records into the default macro, register `@`. `Space m M` asks
  for a register first; any printable key names one.
- Only one recording runs at a time. Starting a second is refused rather than
  replacing the first.
- A recording captures raw input in arrival order and replays it through the
  same dispatch, so a replay does exactly what the typing did.

**Example:** add a semicolon to the end of several lines.

1. `Space m m` — start recording.
2. `A ; Escape j` — append `;`, return to Normal, move down.
3. `Space m m` — stop.
4. `Space m r` — replay on the next line. It accepts a count.

**Replaying:**

- Replay runs between frames, not by holding the input loop.
- `Escape` or `Ctrl-c` cancels the remaining work. Inputs already run are kept,
  because a macro may have done things that cannot be rolled back.
- While work remains, the interaction line shows progress and the cancel keys.

**Safety limits** (reaching any of them stops the whole replay):

- Direct and mutual recursion are refused, naming the register chain.
- Distinct macro calls may nest up to 16 levels.
- One top-level replay has a 10,000-unit budget shared by raw key events,
  characters of literal text, counted repetitions, and nested macros. Counted
  commands expand between frames.
- Range operations whose exact meaning cannot be split are limited to 128
  repetitions in one recorded input, and refused before taking effect when
  larger.

<!-- navigation -->
[Contents](README.md) · [Previous: The screen](the-screen.md) · [Next: Files and buffers](files-and-buffers.md)
