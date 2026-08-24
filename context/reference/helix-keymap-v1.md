# Helix keymap compatibility — V1

Target: [Helix master keymap](https://docs.helix-editor.com/master/keymap.html),
verified 2026-07-27.

Status meanings:

- **Implemented** — registered and dispatched by Runyte.
- **Deviation** — implemented, but intentionally differs from Helix or retains
  an older Runyte shortcut.
- **Planned** — registered for discovery but requires a deferred subsystem.
- **Unsupported** — registered where useful, but excluded from V1.

V4 Phase 0 replaced the single-selection model with multi-range selections.
The rows below marked V4 were previously a single `Unsupported` entry reading
"V1 deliberately has one selection".

V4 Phase 2 added language servers. The goto, symbol, diagnostic, rename, and
code-action rows below were Planned with the reason "requires LSP" until it
landed. Their availability is now unconditional in the registry: a buffer with
no configured or reachable server reports why as a notification rather than
being unbound.

## Single-letter bindings

A single letter in an **editable** buffer must be a Vim or Helix binding, or
carry a row below justifying itself as `Added`. Everything else takes a
namespace path, optionally advertising a short alias on its namespace row the
way `Space / f` advertises `Space f`.

Audited 2026-08-13, the editable-buffer surface holds to this: every active
single letter is standard in one or both editors, except the deviations `0`,
`$`, `X`, and `s` — which take standard letters and change their meaning
rather than claiming new ones — and two genuine additions, `V` and `Y`. `Y` is
not a Helix binding but is Vim's, with Vim's meaning.

A read-only buffer is marked `[RO]` in the pane title and the status line, and
`· Read-only` in its help title, so the three surfaces cannot disagree about
whether a view accepts edits.

Scoped buffers never shadow a global binding. Enter and the few other direct
scoped keys remain only where the same key is globally unbound; contextual Git
operations live behind `Tab`, whose mnemonics own input only while their menu
is open. Help's scoped `q` is also globally unbound.

## Normal and Select movement

| Sequence | Helix command | Runyte command | Status | Notes |
| --- | --- | --- | --- | --- |
| `h`, `Left` | `move_char_left` | `move-left` | Implemented | Extends the anchor in Select mode. |
| `j`, `Down` | `move_visual_line_down` | `move-down` | Implemented | Moves by screen row when `editor.soft_wrap` is enabled and by text line otherwise. |
| `k`, `Up` | `move_visual_line_up` | `move-up` | Implemented | Moves by screen row when `editor.soft_wrap` is enabled and by text line otherwise. |
| `l`, `Right` | `move_char_right` | `move-right` | Implemented | Extends the anchor in Select mode. |
| `w`, `b`, `e` | word start/back/end | matching word motions | Implemented | Unicode-safe; words, punctuation, and whitespace are distinct classes, and a line break ends a word. Unlike Helix, the cursor never rests on a line terminator or an empty row. |
| `W`, `B`, `E` | long-WORD motions | matching long-word motions | Implemented | A WORD is a run of non-whitespace characters. |
| `f`; `F`, `t`, `T` + character | character-find motions | matching find/till motions | Implemented | Searches across lines, like Helix. `Space f` belongs to the project finder, so character find keeps only its direct `f` binding. |
| `Home`, `End` | line start/end | matching line motions | Implemented | Empty-line boundaries are clamped safely. |
| `0`, `^`, `$` | not default Helix bindings | line start/first non-whitespace/line end | Deviation | Vim-style aliases retained alongside Home/End and goto mode. |
| `Ctrl-b`, `PageUp` | `page_up` | `page-up` | Implemented | Page size comes from the active pane. |
| `Ctrl-f`, `PageDown` | `page_down` | `page-down` | Implemented | Page size comes from the active pane. |
| `Ctrl-u`, `Ctrl-d` | half-page up/down | matching half-page motions | Implemented | Moves the cursor and lets rendering follow it. |
| counts, `<n>G`, `<n>gg` | counted motion | counted registry dispatch | Implemented | Counts repeat countable commands; numbered `G` and `gg` address a one-based line. |
| `Ctrl-o`, `Ctrl-i` | jumplist backward/forward | matching jumplist commands | Implemented | Unlike Helix, `n`/`N` deliberately do not record jumps, so repeating a search does not bury its origin. Unix terminals implementing Runyte's requested keyboard disambiguation distinguish `Ctrl-i` from `Tab`; legacy terminals and Windows do not, so forward jumping has no separate key there. On macOS Runyte requests disambiguation without the repeat/release event stream. |
| `Tab` | jumplist forward | contextual actions | Deviation | Opens the registry-backed actions for the selection or row under the caret. Git menus accept their displayed mnemonic only while open; an ordinary language-server buffer requests its code actions. This deliberate use of Tab costs the forward jumplist key on terminals that cannot distinguish it from `Ctrl-i`. |
| `Alt-o`, `Alt-i` | no Helix equivalent | `jump-backward-buffer` / `jump-forward-buffer` | Added | Step to the previous or next entry in a *different* buffer, skipping every position recorded within the current one. Reading a long document records a jump per section, so leaving it with `Ctrl-o` costs one press per section; these cost one. They share the same history as `Ctrl-o`/`Ctrl-i`, so a step lands on a real recorded position rather than the top of the file. Alt rather than Ctrl-Shift because terminals without a disambiguating keyboard protocol report `Ctrl-O` and `Ctrl-o` identically. |
| `Ctrl-s` | `save_selection` | `save` | Deviation | Retains Runyte's original global save shortcut; Helix's `save_selection` has no binding. |

## Changes and selection

| Sequence | Helix command | Runyte command | Status | Notes |
| --- | --- | --- | --- | --- |
| `i`, `a` | insert/append | insert before/after | Implemented | Uses the ordered single selection. |
| `I`, `A` | insert at line start/end | matching insert commands | Implemented | `A` permits insertion after the last character. |
| `o`, `O` | open below/above | matching open-line commands | Implemented | Enters Insert mode. |
| `r` + character | `replace` | `replace-char` | Implemented | Replaces each selected character. The argument is consumed literally, so `r Space` inserts a space without opening command hints; a subsequent Space starts the application command tree (`r Space Space`). Runyte remains in Normal mode while it waits and marks the replacement heads red rather than labelling this one-character argument as Insert mode. |
| `~` | `switch_case` | `toggle-case` | Implemented | Unicode case mappings are supported. |
| `u`, `U` | undo/redo | undo/redo | Implemented | An Insert-mode action is one history checkpoint; selections follow the inverse transaction. |
| `y` | `yank` | `yank` | Implemented | Writes the unnamed register and a selected named register. A bare caret yanks the character under it, the same span `d` and `c` act on, rather than the whole line. A transient `x`/`X` line selection produces a linewise register; an explicit `v` selection remains characterwise even when it spans a whole line. Yanking always returns Select mode to Normal, keeping the selection so `P` still pastes at its start. In a directory buffer a bare caret yanks the whole entry on its row, because that is the identity `p` there transfers. |
| `Y` | not a Helix binding | `yank-line` | Added | Yanks every row the selection touches as whole lines, writing the linewise register `x y` writes without walking the selection through line mode. Vim's `Y` in the shape Vim users expect, and the counterpart to Helix's characterwise `y`. Unlike `x`, it leaves the selection and the caret alone: it copies rather than choosing what to operate on next. |
| `p`, `P` | paste after/before | matching paste commands | Implemented | Reads the selected register, including linewise content. |
| `>`, `<` | indent/unindent | matching indentation commands | Implemented | Uses configured tab width. |
| `Ctrl-c`, Insert `Ctrl-c` | `toggle_comments` | `toggle-comments` | Implemented · Deviation | Comments or uncomments every line the selection touches. The marker comes from a `line_comment` declared per language in `syntax::grammars` rather than from a text heuristic, so it is `//` for Rust, C, C++, Go, Java, JavaScript, TypeScript, TSX, Kotlin, and Swift, and `#` for Python, Bash, TOML, and YAML. CSS, HTML, JSON, Markdown, and a buffer with no detected language have no line comment and are reported as such rather than falling back to a block pair or to a borrowed marker; Helix's `toggle_block_comments` has no counterpart. The marker is inserted at the block's shared minimum indent rather than at each line's own, so relative indentation inside a nested block survives the round trip, and blank lines are skipped in both directions. A recognized first-line shebang is also skipped when it is an extensionless document's only language signal; changing it would discard the language and make the inverse press unavailable, while any other selected rows still toggle. Uncommenting consumes the marker and at most one following space, which means a marker that merely begins with the language's own is treated as a comment: under `//` a Rust doc comment `/// x` uncomments to `/ x`, as it does in Helix. A partly commented block commits to fully commented first, so a second press is always the inverse of the first. The Insert-mode binding is the deviation: Helix binds `Ctrl-c` only in Normal and Select. It joins the Insert `Ctrl-s`/`Ctrl-u`/`Ctrl-k` family and acts on each caret's own line, since entering Insert mode collapses every range to a point and a block can therefore only be chosen in Normal or Select mode. `Ctrl-c` keeps its unrelated cancel meaning in prompts and pickers and its pass-through in Terminal Insert, both of which are handled before the buffer keymap is consulted. |
| `d`, `c` | delete/change selection | matching edit commands | Implemented | Applies to every selection. An empty range represents one character. `d` after a transient `x`/`X` selection writes and removes whole lines, while an explicit `v` selection remains characterwise. |
| `v` | `select_mode` | `enter-select-mode` | Implemented | Toggles Select mode. |
| `x` | `extend_line_below` | `select-line` | Implemented | The first press snaps each range to whole lines; repeated presses extend downward, per selection. Unlike `v`, the resulting selection is transient: any other command ends it and restores the previous mode. |
| `X` | `extend_line_above` | `select-line-up` | Deviation | Mirrors `x` upward rather than Helix's "extend to line bounds": it walks the same edge, so `x x X` leaves one line selected and a further `X` takes the line above. |
| `%` | `select_all` | `select-all` | Implemented | Selects the complete buffer. |
| `;` | `collapse_selection` | `collapse-selection` | Implemented | Collapses every range to its head. |
| `Alt-;` | `flip_selections` | `flip-selection` | Implemented | Swaps cursor and anchor. |
| `"` + register | `select_register` | `select-register` | Implemented | Uppercase register names append; `_` is the black-hole register. |
| `q`, `Q` | macro replay/record | macro commands under `Space m` | Deviation | Both letters are unbound. `Q` and `q` say nothing about what they do, so macros moved to one namespace: `Space m m` starts the default recording and the same keys stop it, `Space m M` and `Space m R` name a register, `Space m r` replays the default, and `Space m l` lists what has been recorded. The default macro is the `@` register. The keys spelling the stop are staged rather than recorded, so they never end up inside the macro they finished. |
| shell keys such as `\|` | shell operations | `shell-pipe` | Unsupported | Process-backed editing is outside V1. |
| `s` | `select_regex` | `search` | Deviation | Runyte spends `s` on its case-insensitive search. Searching with two or more characters selected matches only inside them, which is what `select_regex` was for, so the command was retired rather than rebound. |
| `S` | `split_selection` (regex) | — | Deviation | Unbound. Splitting moved to `Space s e` / `Space s b`, which leave a bare cursor at each line's end or start rather than a range per line, and Runyte no longer has a case-sensitive search flavour for `S` to carry: `(?-i)` in a regular expression asks for one. |
| `,`, `Space s c` | `keep_primary_selection` | `keep-primary-selection` | Implemented | Keeps the primary range of any multi-selection. Search cycling already reduces its result to one match. |
| `Alt-,` | `remove_primary_selection` | `remove-primary-selection` | Implemented | Refuses to empty the selection. |
| `C`, `Alt-C` | `copy_selection_on_next_line` / previous | `copy-selection-down` / `-up` | Implemented | Adds the caret on the nearest row that holds a character at the caret's column, skipping the rows too short for it. A row ending exactly at the column does not hold it and is skipped too: landing there would slide the caret onto the row's last character, and that shifted column would then seed the next `C`. The column is therefore never approximated — `V` is the command that widens short rows instead. |
| `V`, `Alt-V` | not Helix default bindings | `copy-selection-down-padded` / `-up-padded` | Added | Adds a cursor on the immediately following or preceding row, padding short or empty rows with spaces so it reaches the same display column. `Alt-V` mirrors `V` upward exactly as `Alt-C` mirrors `C`; padding for several cursors landing on one short row is collected before any edit, so the row widens once and the whole gesture is a single undo step. |
| `)`, `(` | `rotate_selections_forward` / backward | `rotate-selection-forward` / `-backward` | Implemented | Moves the primary designation only. |
| `Alt-k`, `Alt-j` | `keep_selections` / `remove_selections` | `keep-matching-selections` / `remove-matching-selections` | Deviation | On `Space s k` and `Space s r`; the Alt keys are unbound. Each opens its own `keep (regex):` or `remove (regex):` prompt, as Helix does. They no longer borrow the search pattern: since `s` and `S` accept literals, a pattern typed there is not a regular expression and could not be reused as one. `Alt-j`/`Alt-k` were also the key-hint popup's scroll keys, so leaving them bound kept the popup and the keymap fighting over them. |
| `&` | `align_selections` | `align-selections` | Implemented | Pads to the rightmost selection column, so a column of multicursors lines up. Also on `Space s a`. Padding only inserts spaces, so the rightmost caret is necessarily the target; aligning to the leftmost would mean deleting text. |
| `Alt-_` | `trim_selections` | `trim-selections` | Deviation | On `Alt-_`; Helix binds it to `_`, which Runyte gives to `trim-trailing-whitespace` instead. The pairing follows `;`/`Alt-;` and `,`/`Alt-,`, where the plain key is the common gesture and the Alt key the neighbouring one. A range holding nothing but whitespace collapses to a caret at its start rather than staying as it was, so splitting on lines and trimming leaves a usable cursor on blank rows instead of a range Helix would have left untouched. |
| `_` | no Helix equivalent | `trim-trailing-whitespace` | Added | Deletes trailing spaces and tabs from every line the selection touches, so `%` then `_` strips the whole buffer. Selection-scoped by line rather than by range: what the ranges pick out is which lines to trim, not which characters to remove. Leading whitespace is left alone so indentation survives, which means a line holding only whitespace is emptied outright. Shares its trimmer with the `editor.trim_trailing_whitespace` save hook, so an on-demand trim and a save cannot disagree. |
| `Alt-(`, `Alt-)` | rotate selection *contents* | matching rotate-content commands | Implemented | Applies all replacements as one transaction. |
| `Alt-s` | `split_selection_on_newline` | — | Deviation | Unbound. Splitting a selection is `Space s e` / `Space s b`, and neither produces one range per line. |
| `Space s e`, `Space s b` | no Helix equivalent | `split-selection-at-line-ends` / `-line-starts` | Added | Leaves one collapsed cursor per selected line, at the line's end or first character, for the select-lines-then-type gesture. |
| `Space p w` | no Helix equivalent | `hard-wrap` | Added | Hard-wraps every selection at `editor.hard_wrap_width`. Words remain intact unless one word exceeds the limit. |
| `Space p r` | `:reflow [width]` | `reflow` | Added | Refills each selected paragraph at `editor.hard_wrap_width`. Unlike Helix's generic refill, preserves Markdown block structure and hanging list indentation, refills block quotes behind a repeated `>` leader; in source files only `#` and `//` comment paragraphs are changed, with their leaders repeated. |
| `Space p j`, not `J` | `join_selections` | `join-selections` | Deviation | Removes every line break inside each selection, replacing it with text typed at a `join with (empty joins directly): ` prompt: empty runs the lines together, a space reproduces Helix's `J`, and anything else is inserted literally. The prompt is why `J` stays unbound — a bare letter cannot announce that a delimiter is coming — and why the command sits beside `Space p w`, whose inverse it is. Whitespace against a removed break goes with it, so the first line keeps its indentation and joined lines do not leave a run of spaces. Unlike Helix, a selection covering one line joins nothing rather than pulling up the line below, and the row after the selection is never drawn in — including when a pointer drag's half-open span ends at that row, whose terminator is held out of the change. A selected blank row is still a selected row and joins as an empty piece. |
| `Space p t` | no Helix equivalent | `format-table` | Added | Aligns the columns of the table the selection covers, padding every cell to the widest one in its column. A table is rows opening with `|`, cells divided by unescaped `|`, and at least one separator row of dashes among them. The separator is required — a run of pipe rows alone is as likely to be a closure or a diff hunk — but it is recognized wherever it falls rather than only as the second line, since the selection is a hand-picked run of rows that may open on the separator or close below a footer rule. The drawing is kept: a separator written `+---+---+` keeps its `+` signs, every boundary character stays where it was, and `:---`, `:---:`, `---:` colons survive and decide the column's content alignment. A tab inside a cell is expanded to spaces at `editor.tab_width` stops, because a tab's width is its distance to the next stop and the column it lands on is what the formatter is solving for. Alone among the `Space p` commands it widens each selection to whole rows, since a row is only a row from its opening `|` to its close, and folds together the selections that widening puts on the same or consecutive rows; it still never reaches past the last selected row. Blank lines inside the selection are left as written, rows disagreeing on cell count are squared up with empty cells, and all rows take the indentation of the first. A selection holding a line that is neither blank nor a row, or holding no separator, is refused with `no table detected in the selected lines` and nothing is edited. |
| `Space p s` | no Helix equivalent | `toggle-soft-wrap` | Added | Toggles pane-width soft wrapping for the session. Soft wrapping uses word boundaries and remains dynamic as the pane is resized. |

## Search

Runyte's search deviates from Helix deliberately and as a whole, so the table
below is a summary of one design rather than a list of independent choices.

Two flavours share one behavior. `s` compiles the pattern as an escaped,
case-insensitive literal and `/` as a regular expression; both select **all**
matches at once, each as a forward range so the whole match is selected with the
caret on its last character, where an append or a motion continues from. Helix
instead moves to one match and keeps the
pattern in a register for `s`, `Alt-k`, and `Alt-*` to reinterpret. Escaping the
literal flavour is what makes `foo(` findable without knowing regular
expressions exist, and is why the pattern can no longer be shared with commands
that require one.

Case sensitivity is not a third flavour. It was `S` and `Space s w S` until the
`Space /` rewiring, and both were retired rather than respelled: `(?-i)` in a
regular expression is the whole of what they offered, and keeping them cost a
bare letter and a namespace row each.

Neither flavour takes a namespace spelling. Two keys are already the short
spelling, so `Space s s`, `Space s S`, and `Space s /` were retired with the
duplicates they were: `Space s` is selections only now. `Space /` widens exactly
these letters to the whole project — the sigil says search, the prefix says the
project rather than the file in front of you, and the letter after it is the one
the bare key already uses.

Pristine results render the primary range light orange with one orange cursor;
secondary ranges retain the theme's secondary selection colour and hide their
cursor blocks. A selection motion
ends that search-specific presentation and restores the ordinary endpoint
cursors because subsequent extension depends on them. `i`, `a`, and `c` then
show red Insert-mode cursors at their resulting insertion points; `r` shows all
heads in red while it waits for the replacement character.

A search is confined to the current selection when at least one range covers two
or more characters. A bare caret is a one-character range in this grammar, so
that threshold is what separates a selection from a cursor position. The spans
are remembered in `SearchQuery::region` and mapped through transactions
alongside pane selections, so `n` and `N` keep wrapping inside the region after
the text moves. Successive searches narrow, because the matches of one search
are the region of the next.

| Sequence | Helix command | Runyte command | Status | Notes |
| --- | --- | --- | --- | --- |
| `s` | — | `search` | Added | Escaped literal, case-insensitive. No namespace spelling: two keys are already the short one. |
| `/` | `search` | `search-regex` | Deviation | Regular expression over the whole buffer text, so a pattern may span lines. Also the way to match case, with `(?-i)`. |
| `?` | `rsearch` | — | Deviation | Unbound. Search has no direction to choose once it selects every match; `n`/`N` supply the direction afterwards. |
| `n`, `N` | next/previous match | `search-next` / `search-previous` | Deviation | Select only the next or previous match, wrapping inside the remembered search region rather than at buffer boundaries. The initial search still selects every match for a direct batch edit; cycling changes the intent to a single-match edit. |
| `*` | search current selection/word | `search-selection` | Deviation | Selects every occurrence of the word or selection under the caret, matched as a case-sensitive literal. This absorbs what `Alt-*` did. |
| `Alt-*` | unbounded selection search | — | Deviation | Unbound; `*` covers it. |
| `Space / f` | file picker | `open-file-picker` | Added | Opens the project finder in file mode; Tab switches to open buffers and terminals without clearing the query. A file query ending in `/` matches directories only. `Space f` is the short alias the namespace row advertises; `f` alone remains character find. |
| `Space / g` | — | `open-fuzzy-grep` | Added | Fuzzy-rank file-content lines from the project root and jump to the selected line. |
| `Space / s`, `Space / /` | — | `global-search`, `global-search-regex` | Added | The same two flavours across the workspace. Each opens a retained `[workspace search]` special buffer; Enter on a result is registry-backed and jumps to its typed source range, and the clean result remains available while it is among the two most recently active special buffers. `Space / /` repeats the namespace letter for the flavour reached for most, the way `Space b b` and `Space m m` do. |
| — | file picker at working directory | `open-directory-file-picker`, `open-directory-fuzzy-grep` | Deviation | Unbound. The directory-scoped picker and grep keep only their colon spellings, `:file-picker-directory` and `:fuzzy-grep-directory`, having earned no key in practice. |

The prompt model supports Escape/Ctrl-c, character and word movement, Home/End,
Backspace/Delete, Ctrl-u, and Ctrl-k. Prompts are labelled by flavour —
`search: `, `search (regex): `, and the `workspace search…` variants.

## Goto and view modes

| Sequence | Helix command | Runyte command | Status | Notes |
| --- | --- | --- | --- | --- |
| `gg`, `ge` | file start/end | matching file motions | Implemented | Arbitrary key-sequence dispatch replaces the old pending enum. |
| `gh`, `gl` | line start/end | matching line motions | Implemented | Extends in Select mode. |
| `gs` | first non-whitespace | matching motion | Implemented | Local text operation. |
| `gt`, `gc`, `gb` | view top/center/bottom | matching view motions | Implemented | Uses the active pane viewport. |
| `gp`, `gP` | next/previous paragraph | matching paragraph motions | Implemented | Paragraphs are runs of non-empty lines. Extends in Select mode and accepts counts. |
| `gw` | `goto_word` | `goto-word` | Deviation | Dims the active pane and assigns prefix-free labels to eligible visible words by projected distance from the cursor: nearby targets use one red key and farther targets use two neon-cyan keys of one hue. A two-key prefix narrows to red suffixes at the target cells. Labels never cross a wrap or viewport edge. Extends in Select mode and records a jumplist entry. |
| `gd`, `gD`, `gy`, `gr`, `gi` | LSP goto operations | matching command identities | Implemented | One result moves the selection; several open the shared result picker. |
| `zz`, `zc` | align view center | `align-view-center` | Implemented | Updates presentation-neutral pane scroll state. |
| `zt`, `zb`, `zm` | top/bottom/horizontal middle | matching align commands | Implemented | Existing UI scroll margins can refine the final rendered offset during integration. |
| `zj`, `zk`, arrows | scroll view | matching scroll commands | Implemented | Does not move the selection. |
| `z` + page keys | page/half-page motions | matching page commands | Implemented | Same commands as Normal mode. |
| `Z` + view key | sticky view mode | matching view command | Implemented | The `Z` prefix remains pending until Escape. |
| `mm` | `match_brackets` | `match-bracket` | Implemented | Resolves through the syntax tree, so brackets inside strings and comments are ignored. Requires a known language. |

## Window and Space modes

V7 adds an additive Runyte namespace without withdrawing the established
Helix-style surface. Registry role metadata calls canonical/default bindings
**Primary**, selected short `Space` paths **Fast**, and retained historical
aliases **Compatibility**. These roles do not duplicate command descriptions:
dispatch, help, and which-key hints still obtain the description from the
semantic command inventory. Labelled namespace rows are generated from that
same registry and are not executable exact bindings.

An overflowing key-hint popup scrolls with Up and Down when the pending prefix
does not claim that arrow as a continuation. `Alt-j` and `Alt-k` remain the
unconditional fallback, including for `z`, `Z`, and `Ctrl-w`, whose arrow
continuations must still reach the registry.

The `Space l` namespace is labelled **Language (LSP)** and `Space x` is
labelled **Syntax (Tree-sitter)**. Key hints dim either namespace with its
active-buffer unavailability reason when the corresponding service is not
ready. A dimmed namespace remains navigable: descendant rows carry their own
availability, so LSP manager commands such as status and restart can remain
available when document-specific commands are not.
The `Space g` **Git** namespace uses the same navigable dimmed state when the
Git executable is unavailable or the current project is not in a repository.
The `Space t` namespace is labelled **Terminals** and carries no capability:
its one command that needs a pane showing a terminal reports that itself, and
`Space t s` is meant to be reached from a pane that is showing a document.

Mouse input is grammar-independent application input rather than a hidden key
binding table. Left click focuses/places, Shift-click extends, left drag
selects, wheel events scroll the pane under the pointer, and a drag on a shared
border resizes that split. A click that focuses another pane leaves Insert
mode, while a following drag still selects and enters Select mode. All
coordinates resolve through the prepared fold/wrap row projection; keyboard
overlays retain input ownership while open.

| Sequence | Helix command | Runyte command | Status | Notes |
| --- | --- | --- | --- | --- |
| `Space Space` | not a Helix binding | `repeat-last-space-command` | Added · Primary | Repeats the last successfully invoked command reached through an actual `Space …` sequence against current editor state. The repeat command does not replace its own history, and `Ctrl-w` aliases do not enter it. |
| `:file-picker-directory` | file picker at current working directory | `open-directory-file-picker` | Added · Primary | Recursively fuzzy-finds below the active file's parent or explorer root; a pathless/generated buffer falls back to the working directory. Ancestor ignore rules remain active down from the project root, while Git and Runyte runtime state cannot become scan roots. This deliberately follows buffer context rather than Helix's process working directory. It held `Space s F` and `Space F` until the `Space /` rewiring and now has no key. |
| `Space r` | not a Helix binding | `reload` | Added · Primary | Reloads the active text file from disk or refreshes the active directory explorer. A dirty explorer still requires confirmation. |
| `Space W`; `:session-list`, `:sl` | not a Helix binding | `session-list` | Added · Primary | Opens the persistent-session manager directly. This is one exact binding rather than a namespace; session actions remain behind Tab in the manager, and `Ctrl-t` toggles the selected session's bounded live pane preview. Inside the manager, `1`-`9` attach to the session holding that number while the filter is empty, and are ordinary filter text once anything has been typed. The digits belong to the manager overlay, not to the keymap: `Space W` remains a complete binding with no prefix timeout. |
| `Space :` | not a Helix namespace | — | Removed | Bare `:` is the single command-palette binding. |
| `Space c y/p/P` | not a Helix namespace | clipboard yank / paste after / paste before | Added · Primary | These use the system clipboard; bare `y/p/P` continue to use Runyte registers. |
| `Space l h/c/s/S/d/r/a` | not a Helix namespace | hover / completion / document symbols / workspace symbols / diagnostics / rename / code action | Added · Primary | `Space l c` enters Insert mode and requests completion; its hint advertises Helix's Insert `Ctrl-x` as a mode-qualified alias. The redundant short Space aliases were removed. |
| `Space l f/R/?` | not a Helix namespace | `format` / `lsp-restart` / `lsp-status` | Added · Primary | These bindings target the existing colon identities directly; they do not parse command text or create duplicate editor identities. |
| `Space g B/b/d/D///g/l/r/t/w` | not a Helix namespace | `git-blame-file` / `git-branches` / `git-diff` / `git-diff-side-by-side` / `git-search-commits` / `git-status` / `git-log` / `git-refresh` / `git-stashes` / `git-worktrees` | Added · Primary | Helix has no Space-g namespace, so this claims a free prefix for Git navigation and refresh only. `B` opens full-file attribution, `b` opens branches, `d` opens the active file's patch, `D` compares its complete versions side by side, `/` fuzzy-searches the newest 5,000 commits reachable from `HEAD` — it took `f`, for "fuzzy", until `/` became the sigil for search in any namespace, and commits are the only Git corpus large enough to need one — `g` lists changed files, `l` opens history in pages of up to 10,000 commits, `r` refreshes, `t` lists stashes, and `w` opens the repository worktree list. Both diff views use index-to-working-tree outside the changed-file list and follow the selected staged or unstaged row inside it; an absent side of an added or removed file is hatched filler. Mutations and commit preparation live in the changed-file list's Tab menu; single-line blame remains available only as `:git-blame`. The namespace itself holds no network command; pull and push are bound in the branch and changed-file views, and fetch has none. |
| `Enter` / `Tab n/D` in the branch list | not a Helix scope | checkout-branch / create-branch / delete-branch | Added · Primary | Enter checks out the row under the cursor. The Tab menu creates or deletes the selected branch without taking global `n`; deletion remains separately confirmed. Registered worktree annotations and all existing dirty-state refusals remain attached to the row's stable branch identity. |
| `Enter` / `Tab n/N/D` in the worktree list | not a Helix scope | open-worktree / create-worktree / create-new-worktree / remove-worktree | Added · Primary | In persistent mode, Enter attaches to the selected root's session and starts it when necessary; successful `n`/`N` creation immediately attaches the same way. Standalone mode keeps creation and removal as Git-only actions but refuses attachment. Global search and replace keys remain available, and `Space g r` refreshes the view. |
| `Enter`/`Ctrl-n`/`Ctrl-p` in the Git log | not a Helix scope | open-git-commit / next-git-log-page / previous-git-log-page | Added · Primary | The log is an object-identified, paged application view. Enter opens the selected commit's bounded detail and patch. Paging sits on `Ctrl-n` and `Ctrl-p` so every motion key keeps its meaning in this view; `Space g r` refreshes it. |
| `Tab a/D` in the stash list | not a Helix scope | `git-stash-apply` / `git-stash-drop` | Added · Primary | The Tab menu applies or separately confirms dropping the stable stash row. `Space g r` refreshes; named creation remains in the three explicit colon commands. |
| `Tab s/u` in a per-file Git diff | not a Helix scope | `git-stage-hunk` / `git-unstage-hunk` | Added · Primary | Applies the exact bounded patch under the cursor only after Git's `--check` and repository, HEAD/index, disk, and buffer preconditions still match. The menu mnemonics do not take global search or undo. |
| `Enter` in the blame view | not a Helix scope | open-git-commit | Added · Primary | Blame is computed from the originating buffer's live text and revision. Enter shares the same typed commit-navigation command as the log; an uncommitted row refuses because it has no commit identity. |
| `Tab p/P` in both Git views | not a Helix scope | pull-branch / push-branch | Added · Primary | The two network mnemonics are identical in the branch-list and changed-file action menus without taking the global paste pair. Pull remains fast-forward-only with the existing confirmed rebase offer; push sets an upstream on first publication and never forces. Both retain their bounded asynchronous, no-autostash, no-prompt, and cancellation behavior. |
| `Enter` / `Tab s/u/D/o/S/c/i/p/P` in the changed-file list | not a Helix scope | `git-diff` / `git-stage` / `git-unstage` / `git-discard` / open-changed-file / stage-all-changed-files / `git-commit` / `git-index` / pull-branch / push-branch | Added · Primary | Enter diffs the selected row. The Tab menu lists row-scoped stage, unstage, discard, and open actions first. Buffer-wide `S` stages every unstaged or untracked row, `c` opens a commit message, `i` reviews the index, and `p`/`P` pull and push. `Space g r` remains the global refresh command. Every ordinary letter binding remains unchanged in the read-only list. |
| `Space m m/M/r/R/l` | not a Helix namespace | record default / record named / replay default / replay named / list macros | Added · Primary | `Space m m` is the whole start-and-stop gesture for the default `@` macro, so no second binding has to be remembered to finish a recording. `Space m M` and `Space m R` take the register as the next key. Starting a second recording while one is open is refused. |
| `Space l g d/D/y/r/i` | not a Helix namespace | definition / declaration / type definition / references / implementation | Added · Primary | `gd/gD/gy/gr/gi` remain Primary established modal bindings. |
| `Space x e/s/p/c/h/l` | not a Helix namespace | syntax expand / shrink / parent / child / previous sibling / next sibling | Added · Primary | Presentation-neutral structural commands; no syntax implementation is duplicated in the grammar. Expand retains Tree-sitter's half-open bounds and shrink restores the prior coordinate semantics. Parent, child, and sibling selections put the block cursor on the last included character while retaining exact syntax bounds for edits and repeated navigation. |
| `Space x o` | not a Helix namespace | `outline` / `document-outline` | Added · Primary | Opens the immediate Tree-sitter outline without requiring an LSP server; Enter resolves a revision-safe syntax target. |
| `Space x a f/c/p` | not a Helix namespace | select function / class / parameter around | Added · Primary | The `a` row is a generated namespace, not an exact binding. The block cursor rests on the last included character. |
| `Space x i f/c/p` | not a Helix namespace | select function / class / parameter inside | Added · Primary | The `i` row is a generated namespace, not an exact binding. The block cursor rests on the last included character. |
| `Space x a/i (/[/{/</"/'/\`` and closing-bracket aliases | `ma/mi` plus a surround character | select around / inside an enclosing delimiter pair | Added · Primary | Resolves through the syntax tree, so nested pairs remain structural and bracket-like characters in unrelated strings or comments do not interfere. The block cursor rests on the last included character, matching ordinary Select mode, while edits retain the syntax object's exact bounds. Quotes use their syntax-node boundaries. Ordinary Markdown prose uses a balanced, escape-aware fallback bounded to its enclosing Markdown syntax node because its parser does not represent punctuation pairs structurally; injected code still uses its own syntax tree. |
| `Space x a m`, `Space x i m` | `mam`, `mim` | select around / inside the closest delimiter pair | Added · Primary | Chooses the innermost supported delimiter node without requiring its character and uses the same last-included-character cursor convention. |
| `Space x [ f/c/p`, `Space x ] f/c/p` | not a Helix namespace | previous / next function, class, or parameter | Added · Primary | Bracket rows remain explorable prefixes with no exact-prefix ambiguity. |
| `Ctrl-w w`, `Ctrl-w Ctrl-w` | rotate view | `next-window` | Implemented · Compatibility | Cycles layout order. |
| `Ctrl-w v`, `Ctrl-w Ctrl-v` | vertical split | `split-vertical` | Implemented · Compatibility | Side-by-side panes. |
| `Ctrl-w s`, `Ctrl-w Ctrl-s` | horizontal split | `split-horizontal` | Implemented · Compatibility | Stacked panes. |
| `Ctrl-w h/j/k/l` | directional focus | matching focus commands | Implemented · Compatibility | Ctrl-key and arrow suffix aliases are also registered. Refused while a pane is maximized by `:zen` or `:fullscreen`, as `next-window` already was: the maximized pane is the only pane keys can reach, so the refusal is stated rather than left to fall out of the frame's geometry. |
| `Ctrl-w c` | close window | `close-window` | Implemented · Compatibility | Quit-shaped `Ctrl-w q`, `Ctrl-w Ctrl-q`, and `Space w q` aliases were removed; `Space w c` remains canonical and `:window-close` / `:wc` is its typed spelling. |
| `Ctrl-w o` | only window | `only-window` | Implemented · Compatibility | Keeps buffers but removes other panes. |
| `Space w f`, compatibility `Ctrl-w f` | not a Helix binding | `toggle-fullscreen` | Added · Primary | Toggles the active pane across the whole editor area with its ordinary content layout; the split tree stays intact underneath. Normal and Select only, like close and only-window, and typed as `:fullscreen`. |
| `Space w z`, compatibility `Ctrl-w z` | not a Helix binding | `toggle-zen` | Added · Primary | Keystroke spelling of `:zen`, which until now existed only as a typed command. It shares one state with `toggle-fullscreen`: asking for the other maximized view while one is showing switches to it rather than stacking a second maximization, and only the view showing toggles off. |
| `Space w` + window key | window mode | matching window commands | Implemented · Primary | Three-key registry sequences; no hard-coded submode. |
| `Ctrl-h/j/k/l` | not a Helix binding | matching focus commands | Added · Fast | Off unless `editor.fast_pane_keys` is `true`, in which case the four keys reach the same focus commands as their `Ctrl-w` prefixed spellings, in Normal, Select, Insert, and Terminal Insert alike. Both spellings focus the destination in the same keystroke without capturing review: a terminal destination starts live Insert and a document reached from Terminal Insert starts Normal. It is a whole second registry rather than a dispatch exception, so key execution, help, and hints agree about what the keys do. Turning it on shadows Insert `Ctrl-j` (`insert-newline`) and Insert `Ctrl-k` (`delete-to-line-end`), which leave the registry rather than merely losing dispatch, and removes all four keys from a terminal child. Both prefixed spellings are unaffected. |
| `:resize-right/left/top/bottom +/- N` | not a Helix binding | pane resize commands | Added | Moves the named active-pane boundary by `N` terminal cells. `+` grows the active pane and `-` shrinks it; the existing minimum drawable extents still apply. |
| `:path` | not a Helix binding | `path` | Added | Opens a read-only popup with the active buffer's absolute path, wrapped, for both file and directory buffers. `Tab` opens a nested mnemonic menu offering `s` (copy to the system clipboard) or `r` (copy to the unnamed Runyte register); `j`/`k`/arrows/Shift-Tab move the highlight and Enter runs it. Escape backs out one level at a time: out of the action menu first, then out of the popup. |
| `Space e` | explorer | `open-explorer` | Implemented · Fast | Opens the active buffer's directory as an editable directory buffer. From a file buffer it selects that file, so Enter returns to the same buffer. A directory buffer uses its own directory; a pathless buffer falls back to the working directory. |
| `Space E` | explorer | `open-working-directory-explorer` | Added · Fast | Opens the editor working directory as an editable directory buffer. `:cd <path>` changes that directory and retargets an active explorer without changing the stable project root. |
| `Space / f`, `Space f` | project finder | `open-file-picker` | Changed · Primary | Opens in native recursive file mode at Runyte's stable project root, retaining ignore-aware walking, path scoring, highlighting, preview, directory-only trailing `/`, and file/directory activation. Tab/Shift-Tab switch to a combined open-buffer and terminal mode while preserving the query and allowing the file scan to continue. That mode previews authoritative in-memory buffer text or bounded recent terminal output, and shares the file mode's `Ctrl-t` preview toggle. Resource search covers buffer structural names and paths plus terminal names, program/title, stable ID, and current/initial directories; absolute, project-relative, `~/`, and basename path spellings work. `terminal`/`term` and `buffer` softly rank that type first rather than filtering the other type. Enter switches to the chosen buffer or terminal. The existing buffer and terminal managers remain separate action surfaces. |
| `Space / g`, `:fuzzy-grep-directory` | — | `open-fuzzy-grep`, `open-directory-fuzzy-grep` | Added · Primary | Native asynchronous, ignore-aware content pickers rooted at the project or active directory. They fuzzy-rank non-empty UTF-8 lines, prefer authoritative unsaved buffer text over disk, display path and line identities, and open the selected match without invoking an external grep or finder. Candidates are bounded at 10,000 lines and files over 4 MiB are skipped. |
| Directory Enter | open entry | `open-directory-entry` | Implemented | Enter is the only opening key; `e` was withdrawn so a directory buffer keeps the word-end motion every other buffer has. |
| Help `q` | no Helix equivalent | `close` | Added | Scoped to the read-only general `[help]` manual and contextual `[view help]` buffer. `q` and `Q` stay unbound everywhere else, as the macro row below records; help is read-only and row-oriented, so the letter costs nothing here and matches what Vim and Helix bind in their own help. |
| `Space ?`; `:help [topic]`, `:? [topic]` | no Helix equivalent | `help` | Deviation | `Space ?` opens contextual read-only help for the active buffer type, with scoped bindings, prefixes, and direct keys generated from the keymap registry. `:help` / `:?` opens the separate general `[help]` manual, and an optional topic such as `regex` positions that retained special buffer at its section. Both scroll, search, split, close, and remain jumpable while they are among the two most recently active clean special buffers. Contextual help remains one document per buffer type rather than per mode; `normal_and_select_bind_the_same_sequences` in `keymap.rs` fails if that assumption stops holding. |
| Directory `-`, Backspace | parent directory | `open-parent-directory` | Implemented | Retargets the pane's one explorer and selects the child directory just left, so Enter immediately returns to it. If that child is filtered from the parent listing, the parent's saved view remains intact. |
| Directory `Space r` | refresh directory | `reload` | Added · Primary | Refreshes the active explorer through the shared reload binding and requires confirmation before discarding dirty directory edits. Bare `r` remains the normal-buffer replace-character command. |
| Directory `.` | not a Helix binding | `toggle-hidden-files` | Added | Shows or hides dotfiles in every clean explorer, flipping the session value of `editor.show_hidden_files` without writing it to `config.yaml`. Nothing else claims `.`: Runyte has no repeat-dot. A hidden entry is absent from the baseline as well as the listing, so it is neither planned as a deletion nor reported as a change; a dirty explorer refuses the toggle, since re-reading would discard edits that never reached a write plan. |
| Directory `x y` / `x d`, then `p` | Helix selection, yank/delete, paste | existing modal commands | Implemented | The register retains file identities across explorer navigation and panes, so `:w` reviews a copy or move rather than creating an empty file. No Oil-specific binding is added. |
| Directory `Ctrl-w v/s`, `Space w v/s` | split-open entry | `split-vertical` / `split-horizontal` | Changed | The directory scope no longer shadows the split sequences. Splitting an explorer shows the same listing in both panes, exactly as splitting a file shows the same text, and it no longer depends on a row having an entry on it. The new pane still takes an explorer of its own as soon as it navigates, which is what keeps a copy across two explorers possible. |
| `Space v` | a different Helix action | — | Removed | Vertical split remains under `Space w v` and compatibility `Ctrl-w v`. |
| `Space h` | a different Helix action | — | Removed | Documentation remains under `Space l h`; horizontal split remains under `Space w s` and compatibility `Ctrl-w s`. |
| `Space q`, `Ctrl-q` | not Helix defaults | — | Changed | V0 quit shortcuts removed in every mode, including prompts. Leaving the editor is a typed decision: `:q[!]` closes the active pane and a buffer displayed only there, and leaves only from the last pane, while `:qa[!]` always requests an editor exit. No quit spelling terminates a terminal; standalone exit is refused until every child has exited or been explicitly closed in `:terminals`, while persistent mode detaches. `:detach` names that persistent-only operation directly and preserves every pane, buffer, edit, and terminal without a force form. |
| `Space ?` | command palette | `show-help` | Deviation | Retains Runyte's contextual view help; `:` opens its command palette. The palette spelling `:? [topic]` is an alias of `:help [topic]` and opens the separate general manual. |
| `Space w` save | not a Helix default | — | Changed | V0 shortcut removed because `Space w` is now window mode. Use `Ctrl-s`, `:write`, or `:save`. `:save` and `:save!` are Runyte aliases of `:write` and `:write!` with no Helix counterpart, for people who reach for the verb rather than Vim's spelling; `:w` and `:w!` are unchanged. |
| `Space b b` | buffer picker | `open-buffer-picker` | Implemented · Primary | Reuses the shared filterable result picker and previews bounded authoritative in-memory text, toggled with `Ctrl-t`. Explorer rows read `[explorer] dirname` plus a project-relative path (normally `.` at the project root); files and other types retain their structural names and path columns. The two most recently active clean special buffers remain discoverable and jumpable; activating a third retires the least recent detached one. Dirty special buffers remain discoverable, while empty clean scratch buffers retire when no pane displays them. Helix reaches it at `Space b`; Runyte made `Space b` the Buffers namespace and doubled the letter, as `Space m m` does. |
| `Space b c` | not a Helix binding | `close` (alias `c`) | Added · Primary | Safely closes the active buffer without changing the pane layout. Each affected pane uses its own most-recent-buffer history, then any other live buffer, or receives a scratch buffer when none remains. Unsaved text is refused; typed `:close!` / `:c!` is the explicit discard path. `:buffer-close`, `:bc`, `:close-buffer`, and `:cb` remain compatibility aliases. A terminal is not a buffer and is explicitly refused. |
| `Space b n` | no Helix binding | `new-buffer` (`:buffer-new`, alias `new`) | Added · Primary | Opens a fresh pathless scratch buffer in the current pane and records the previous buffer in that pane's history and jumplist. Helix's `:new` is kept as the alias, but its `:n` short form is not, because a single letter is too cheap for a command that discards nothing. |
| `Space / /` | global search | `global-search-regex` | Implemented · Primary | Searches UTF-8 workspace files and authoritative unsaved open buffers. The literal flavour is `Space / s`; see [Search](#search). |
| `Space /`, `Space s` | — | namespaces | Added | "Search the whole project" and "Selections". `Space /` replaced the `Space s w` namespace and the bare `Space /` alias it carried. |
| `Space m` | — | namespace | Added | "Macros". |
| `Space t` | — | namespace | Added | "Terminals". Helix has no terminal, so nothing is displaced. Every command under it is Runyte's own. |
| `Space t n`; `:terminal [command]`, `:term`, `:t` | no Helix equivalent | `open-terminal` | Added · Primary | Runs a program on a pseudoterminal in the active pane. With no argument it runs `$SHELL`; an argument is a command line split the way a shell splits it. Unix only: Windows needs ConPTY, and `context/issues/windows_support.md` already records that a feature is disabled there rather than implemented unsoundly. |
| `:terminal-file-directory`, `:terminal-directory-root`, `:terminal-selected-directory`, `:terminal-session-directory <id\|name>` | no Helix equivalent | contextual terminal creation | Added | Explicitly chooses the active file parent, explorer root, selected directory entry, or another terminal's validated OSC 7 directory without changing bare `:terminal` semantics. |
| `Space t t`; `:terminals` | no Helix equivalent | `open-terminal-list` | Added · Primary | Opens the workspace terminal manager with stable IDs, names, child-title detail, safe directory, unread, and bell state. It contains live sessions only: a child exit removes its session and reveals that pane's most recent buffer, or a scratch buffer, without closing the pane. Showing a session already visible elsewhere moves its single view to the active pane and reveals the old pane's buffer, so two rectangles never race to resize one PTY. Presented session names carry a `[terminal]` prefix, and the active pane title adds `[insert]` while keys reach the child, NORMAL being unmarked; those decorations are not part of the name accepted by typed commands. `:terminal-show <id\|name>` and `:terminal-rename <name>` complete the typed surface. Close in the manager's Tab menu is the only editor action that terminates a child; `:close[!]` and every quit spelling refuse to do so. Terminals are not buffers and remain absent from `Space b b`. |
| `Space t r`; `:terminal-rename <name>` | no Helix equivalent | `rename-terminal` | Added · Primary | Opens the active terminal's rename prompt; the terminal manager exposes the same action through Tab. |
| `Space t q` | no Helix equivalent | `leave-terminal` | Added · Primary | Shows the pane's buffer again and leaves the program running. No colon spelling: it is a property of a view rather than an action on the editor, and every other way of pointing a pane at a document does the same thing. |
| `Space t y`; `:terminal-output` | no Helix equivalent | `copy-terminal-output` | Added · Primary | Freezes the session's screen and history into an ordinary read-only generated buffer, where search, multiple selections, and yank work on real text. This is the honest answer to what Normal mode over a live terminal cannot offer: the cells are a picture of the child's text, not the text. Under the alternate screen there is no history behind the visible screen, and the status line says so. |
| `Space t s`; `:terminal-send [id\|name]` | no Helix equivalent | `send-to-terminal` | Added · Primary | Sends the selection — or the whole buffer when nothing is selected — as one bracketed paste. The convenient visible/last/only default remains, while an exact stable ID or unambiguous name makes targeting deterministic. |
| Terminal Insert, ordinary keys | — | sent to the child | Added | `Escape`, `Ctrl-c`, `Ctrl-o`, `Space`, text, and ordinary control keys reach the program unchanged. The deliberate exceptions are the staged Normal/review key `Ctrl-\` and the restricted `Ctrl-w` window prefix, joined by `Ctrl-h/j/k/l` while `editor.fast_pane_keys` is on. |
| Terminal Insert `Ctrl-w h/j/k/l`, control/arrow suffixes, `w`/`Ctrl-w`, `v`/`Ctrl-v`, `s`/`Ctrl-s` | — | direct pane navigation and splitting | Deviation | `Ctrl-w` begins a restricted declarative namespace without sending a byte. Directional focus and `w` move immediately, starting live Insert on a terminal destination and Normal on a document destination; `v`/`s` create a side-by-side/stacked document pane while the child stays live in its original pane. Directional focus and splitting never capture terminal review. Canceling the prefix leaves the source terminal Insert. Close and only-window continuations remain unavailable here. |
| Terminal `Ctrl-\` | — | leave terminal input / enter review | Added | From INSERT the first press returns to live NORMAL without freezing output; from live NORMAL the second press captures the bounded review snapshot. Further presses remain in review, and `i` returns to input. `Escape` cannot be the exit because full-screen programs and agents need it. **Two spellings.** A terminal implementing the kitty keyboard protocol reports the character; a legacy one has only the control byte `0x1c`, which Crossterm decodes as `Ctrl-4` from the historical table. Runyte requests disambiguated Ctrl keys on macOS while deliberately leaving repeat/release reports off there; terminals without the protocol retain the legacy spelling. Both spellings therefore reach this command. This joins `Ctrl-i`/`Tab` in the list of ambiguities the outer protocol was meant to fix but cannot require every terminal to resolve. |
| Terminal `i`, `a`, `I`, `A`, `o`, `O` | — | `enter-insert-mode` and its relatives | Deviation | All six type again rather than opening a line or moving first, and all six return the view to the live screen. There is no offset to open a line at. |
| Terminal Normal motions; `v`; `x`/`X`; `C`/`Alt-C` | — | terminal review navigation and selection | Added | A second `Ctrl-\`, or the first review operation from live Normal, captures bounded immutable retained output. Character-find, word, line, vertical, paragraph, full-page, and half-page motions move every visible caret and keep the primary one inside `editor.scroll_offset`; `v` enters Select mode and the same motions extend every selection. `gg`/`ge` address the oldest/newest captured rows, and `gw` applies the ordinary visible-word labels to terminal cells. `x`/`X` use Runyte's transient whole-line selection, and `C`/`Alt-C` add carets below/above at the same occupied terminal-cell column while skipping short rows. `Escape` collapses every review selection to a caret and returns to Normal, whichever key made it, as it does over a file. The alternate screen contains only its captured visible screen. |
| Terminal `s`, `/`, `n`, `N`, `)`, `(` | — | terminal review search | Added | Searches the Normal-mode snapshot literal case-insensitively or by regular expression and moves stable highlighted matches while live output continues behind it. |
| Terminal `y`, `Space c y`, `p`, `P`, `Space c p`, `Space c P` | — | register/system copy and terminal paste | Deviation | `y` copies the caret character or all current review selections, newline-separated, to the selected Runyte register; `Space c y` is the explicit system-clipboard copy. Line selections retain their terminating newline. `p`/`P` send the selected Runyte register and enter Insert so the next key reaches the child. `Space c p`/`Space c P` send the system clipboard and stay Normal. Both discard review and paste at the live child's real cursor; before/after has no terminal meaning. Buffer `p`/`P` retain their ordinary Normal-mode behavior. |
| Terminal `u` | `undo` | terminal paste undo | Deviation | A terminal has no transaction log to roll back, so `u` does the only thing an editor can honestly do about text that is already the child's: it sends one delete per character last sent, which a line editor at a prompt answers by erasing exactly the paste. It is offered only while that paste is still the child's last input — any key typed into the terminal ends it, and one undo per paste — and it is refused for an unbracketed paste that carried a line break, since the line discipline has already run what it saw. `U` (`redo`) has no terminal meaning and stays refused. |
| Terminal, everything else editing-shaped | — | refused | Added | Commands that would transact against the hidden document remain refused. Project-wide search, windows, splits, pickers, Git, help, and colon commands are unaffected. |

## Insert, picker, and prompt

| Sequence | Helix command | Runyte command | Status | Notes |
| --- | --- | --- | --- | --- |
| Insert `Escape` | normal mode | `enter-normal-mode` | Implemented | Commits the Insert action as one undo checkpoint and clamps the cursor to Normal semantics. |
| Insert `Ctrl-\` | no Helix default | `enter-normal-mode` | Added | Leaves Insert for Normal in both buffer and terminal panes. Legacy terminals report `Ctrl-\` as `Ctrl-4`, which is accepted as a compatibility spelling. Repeating it is idempotent over a buffer; over a live Normal terminal it explicitly enters review. |
| Insert `Ctrl-w` + pane suffix | no Helix default | matching window command | Added | Begins a restricted window namespace without first leaving Insert. Directional and next-pane continuations are available in every Insert view; terminal panes additionally allow vertical and horizontal splits. Close and only-window remain Normal-only. |
| Insert `Alt-Backspace` | delete previous word | `delete-word-backward` | Implemented · Primary | Unicode-safe and can join lines. |
| Insert `Alt-Delete` | delete next word | `delete-word-forward` | Added · Primary | Deletes the current word, or intervening whitespace plus the next word. |
| Insert `Ctrl-u`, `Ctrl-k` | delete to line start/end | matching delete commands | Implemented | Half-open buffer range edits. |
| Insert Backspace | delete previous character | `delete-char-backward` | Implemented | Existing Backspace behavior retained. |
| Insert Delete | delete next character | `delete-char-forward` | Implemented | Joins the next line at line end. |
| Insert `Ctrl-j`, Enter | insert newline | `insert-newline` | Deviation | Always preserves the row's exact leading indentation. With `editor.smart_newline` enabled (the default), it also uses syntax indentation where available and gives unordered, decimal, alphabetic, and canonical uppercase Roman-numeral list items a hanging indent aligned under their content. Disabling it keeps only the existing indentation. |
| Insert `Tab` | smart tab | `insert-tab` | Deviation | Inserts spaces to the next configured tab stop; no syntax-aware indentation. |
| Insert `Shift-Tab` | insert tab | `insert-literal-tab` | Implemented | Inserts a literal tab. |
| Insert `Ctrl-x` | `completion` | `trigger-completion` | Implemented | Alias advertised by `Space l c`. The explicit session filters the server response by the identifier already before the caret, honors LSP `filterText`/`sortText`, and remains authoritative through further typing and Backspace/Delete until space, newline, acceptance, dismissal, caret movement, or another editing command. Zero matches hide the popup without allowing Word or Path to take over; `.`/`:` refresh the LSP context. Language completion is also requested automatically after `.` and `:` without creating this pinned session. Typing `/` outside an explicit session opens automatic path completion for a valid filesystem directory. Word completion has no trigger key: once a prefix reaches `editor.word_completion_minimum` characters (3 by default), it offers words from open buffers, but never overrides Language or Path. Not a Helix concept. |
| `:` path arguments | Helix command-line completion | command palette path hints | Implemented · Deviation | After a path-valued command and its separating space, the palette lists bounded filesystem matches from the editor working directory or the absolute path being typed. Directories sort first and retain a trailing separator so Tab can descend; `:open` opens a selected directory as Runyte's editable explorer. `~` expands to the user's home directory, and typing a dot prefix reveals matching dotfiles. |
| Insert arrows/Home/End/Page | movement | matching motions | Implemented · Primary | Direct Insert-mode movement bindings. |
| Insert `Ctrl-s` | undo checkpoint | `save` | Deviation | Retains Runyte’s global save shortcut. |
| `Space o o`, `Space o t`, `Space o s` | configuration menu | `open-settings`, `open-theme-settings`, `service-health` | Added · Primary | Settings open as the searchable read-only `[config]` buffer; Enter opens a typed or finite-choice popup and persists only on Enter. The read-only health picker reports syntax and LSP failures without requiring those services. |
| Picker `Ctrl-p`; directory/content picker `Shift-Tab` | previous entry | direct picker action | Implemented | Up remains an alias; printable `k` filters. Shift-Tab instead switches project-finder modes. |
| Picker `Ctrl-n`; directory/content picker `Tab` | next entry | direct picker action | Implemented | Down remains an alias; printable `j` filters like other text. Tab instead switches project-finder modes. |
| Picker page/Home/End | navigation | direct picker action | Implemented | Paging is clamped. |
| Picker `Ctrl-s`, `Ctrl-v` | horizontal/vertical open | split and open | Implemented | The fuzzy file picker applies these actions directly to its selected file. |
| Preview picker `Ctrl-t` | toggle preview | direct picker action | Added | File previews read at most the first 64 KiB (including for large files). Buffer-manager and combined-finder previews use bounded authoritative in-memory buffer text; terminal previews use bounded recent output. Every preview is hidden automatically on narrow terminals. |
| Picker Escape, `Ctrl-c` | close picker | close picker | Implemented | Printable `q` is ordinary fuzzy-filter text. |
| Task Center Up/Down, `Ctrl-p`/`Ctrl-n` | contextual picker navigation | direct Task Center action | Implemented | Moves through filtered task/session/action rows or the open action menu. |
| Task Center Tab, Enter | contextual actions/open | direct Task Center action | Implemented | Tab opens actions; consequential choices prefill a canonical `task-` command and require a separate Enter. |
| Task Center `A`, `Ctrl-r`, Escape | scope/refresh/close | direct Task Center action | Implemented | `A` toggles current/all workspaces; refresh preserves filter and stable selection. |
| Result picker Up/Down, `Ctrl-p`/`Ctrl-n`, Tab | navigation | direct picker action | Implemented | Tab advances in symbol, reference, diagnostic, and code-action pickers; in buffer and workspace lists it opens contextual actions; in the theme choice list it cycles the shown group between every theme, the dark ones, and the light ones. |
| Result picker Enter, Escape, printable keys | select/close/filter | direct picker action | Implemented | Typing filters; Enter jumps to a location or runs an action. |
| Config buffer Enter | choose or type a value | `activate-setting` | Added | Every wrapped row retains its setting identity. Finite values use a list popup; numeric and unbounded string values use a typed popup. Restart-required values are saved without pretending the running service changed. |
| Buffer picker Enter, Tab | open/contextual actions | direct buffer picker action | Added | Enter opens. Tab offers only valid Save, Discard, or Close actions; explorer buffers expose none. Discard requires confirmation and Close never discards dirty text. |
| Completion `Ctrl-n`/`Ctrl-p`, arrows, Tab, Escape | navigate/accept/dismiss | direct popup action | Implemented · Deviation | Overlay handler, like the file picker and text prompts. Only Tab accepts, for every source: a completion popup can open on its own (Language after `.`/`:`, Path after `/`, Word for any three-character prefix), so Enter is kept a plain newline everywhere rather than risk swallowing one. The popup titles itself "LSP Complete" for a language-server response and "Complete" otherwise. |

The Task Center is command-first through `:task-list`. The compatibility
review found no unreserved default sequence that should displace a Helix
binding, so it adds no global key binding.

## Architectural notes

- Normal, Select, and Insert modal execution queries `Keymap`; sequences are
  not duplicated in `App`.
- A pending key sequence is `KeySequence` data of arbitrary length.
- Exact, prefix, exact-and-prefix, invalid, Escape, and Backspace states are
  deterministic.
- Planned and unsupported bindings use the same registry and command metadata
  as implemented bindings.
- Picker and text-prompt editing remain overlay-specific handlers because
  picker/prompt are not editor `Mode` values in V1. The V4 Phase 2 result
  picker and completion popup follow that same precedent: their entry points
  are registry commands, and only their internal navigation is overlay-local.
- A terminal is a pane content type, not a `BufferKind`. `Pane::terminal`
  names the live session a pane shows *instead of* its buffer, and `buffer`
  keeps its ordinary meaning as the document that pane returns to. Nothing in
  `src/terminal/` is a rope, a transaction, or an undo group, because a
  terminal has no answer to what any of them ask.
- Terminal Insert sends keys directly except for `Ctrl-\` and `Ctrl-w`, plus
  `Ctrl-h/j/k/l` while `editor.fast_pane_keys` is on.
  `Ctrl-\` reaches `enter-normal-mode`; `Ctrl-w` enters the restricted window
  grammar used by buffers. `BindingScope::Terminal` adds only terminal-safe
  continuations, keeping execution, help, and hints in one source of truth.
- The rename prompt is a fourth `PromptKind` reusing the search prompt's
  editing model rather than a new text-entry surface. The search, workspace
  search, and selection-filter prompts carry their flavour in the `PromptKind`
  variant, so the label follows the command that opened it without a second
  piece of state to keep in step.
