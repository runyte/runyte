# One search entry point across files, buffers, and terminals

Project-wide search is currently spread across several bindings with different
corpora, and no single entry point covers everything the editor holds open.

| Key | Command | Corpus |
| --- | --- | --- |
| `s` | `search` | the current buffer, escaped literal, ignoring case |
| `/` | `search-regex` | the current buffer, as a regular expression |
| `Space / s` | `global-search` | workspace files and authoritative unsaved buffer text |
| `Space / /` | `global-search-regex` | the same, as a regular expression |
| `Space / g` | `open-fuzzy-grep` | file content lines under the project root, preferring unsaved buffer text |
| `Space / f`, `Space f` | `open-file-picker` | file paths; Tab switches to a combined open-buffer and terminal mode that matches buffer names and paths and terminal names, program and title, stable ID, and directories |

Two gaps follow from that table. Terminal output is not searchable by content
anywhere in the editor. Buffer content is searchable only through the file that
backs a buffer, so a generated or pathless buffer's text is reachable by name
but not by what it contains.

The name search that does span files, buffers, and terminals is reached by a
three-key sequence or its `Space f` alias, and the content search that spans the
project is a different sequence over a narrower corpus.

## Expected behavior

One finder searches anything the editor can reach, in two modes that Tab moves
between without clearing the query.

Its **name mode** ranks files under the project root, open buffers, and terminals
in one merged list. It opens in this mode. Files join the corpus the resource
mode already matches: buffer structural names and paths, terminal names,
program and title, stable ID, and current and initial directories, with
absolute, project-relative, `~/`-prefixed, and basename path spellings. The
soft type preference that ranks `terminal`, `term`, or `buffer` first rather
than filtering extends to files. Today's file and resource modes stop being
separate stops, because Tab now carries the mode switch.

Its **content mode** ranks content lines from those same three sources: file
content lines under the project root, buffer text including buffers with no
file, and terminal output. Tab switches to it from name mode and back, and the
query survives the switch, as it does across the finder's current two modes.

Enter activates the selected result according to its kind: a file opens, a
buffer becomes current, a terminal is focused. A content result additionally
reveals the matched line, which for a terminal means scrolling its scrollback to
that row.

## Expected bindings

| Key | Command | Meaning |
| --- | --- | --- |
| `Space / /`, alias `/` | `open-file-picker` | the finder, in name mode |
| `s` | `search` | escaped literal, this buffer — unchanged |
| `S` | `search-regex` | regular expression, this buffer — moved off `/` |
| `Space / s` | `global-search` | escaped literal, the workspace — unchanged |
| `Space / S` | `global-search-regex` | regular expression, the workspace — moved off `Space / /` |

`Space / f`, `Space f`, and `Space / g` become unbound. `open-file-picker` and
`open-fuzzy-grep` remain in the command inventory with their `:file-picker` and
`:fuzzy-grep` spellings, the second opening the finder directly in content mode,
in the same way that `:file-picker-directory` and `:fuzzy-grep-directory`
already keep colon spellings and no key. The directory-scoped pair is unaffected
and stays files-only below the active directory.

Two properties of the existing namespace decided this shape. `Space /` widens
exactly the letter the bare key already uses, so every leaf under it is the
project-wide reading of a bare key; a buffer-scoped leaf such as `Space / r`
would contradict the prefix's stated meaning. And bare `S` is unbound — it
carried the case-sensitive search flavour until the `Space /` rewiring and was
retired when `(?-i)` replaced it — so the regular-expression flavour displaced
from `/` lands on a free key that already read as a search variant. In-buffer
search therefore keeps both flavours: `s` for the literal one it already had,
`S` for the regular expression.

The command identities do not change, to avoid renaming across the palette,
help, key hints, and the tests that pin the inventory. Their palette
descriptions do: `open-file-picker` describes the finder rather than a file
picker, and `open-fuzzy-grep` describes file, buffer, and terminal contents. A
rename remains available if the retained wording proves misleading in the
palette.

## Result presentation

The finder is a live fuzzy picker in both modes, with the ranking, preview, and
`Ctrl-t` preview toggle it has today.

`global-search` and `global-search-regex` keep their retained `[workspace
search]` special buffer, with registry-backed Enter on a result jumping to its
typed source range, and the clean result remaining available while it is among
the eight most recently active special buffers. Fuzzy ranking stays ephemeral
and an exact pattern keeps producing a durable, reviewable list; the two
surfaces answer different questions and both remain.

## Terminal and buffer content corpus

Terminal content search reads the decoded text of every open terminal session,
live or exited: its scrollback, bounded at `SCROLLBACK_LIMIT` rows per session
by `src/terminal/grid.rs`, together with the current screen. An exited session
keeps its last screen readable until it is closed, so it stays searchable on the
same terms as a live one and needs no separate rule. No further bound is
introduced: the per-session scrollback limit, the workspace cell budget, and the
picker's own candidate budget already bound the corpus.

Buffer content search reads authoritative in-memory text for every open buffer,
including buffers with no path.

## Constraints

- Key dispatch, help, and key hints must continue to read from the one keymap
  registry in `src/keymap.rs`.
- A file that is also an open buffer must produce one result, not two, and the
  authoritative unsaved buffer text must win over what is on disk, as the
  existing content picker already arranges.
- Content search must stay asynchronous and bounded. The current picker bounds
  candidates at 10,000 lines and skips files over 4 MiB, and workspace search has
  its own file-size limit; widening the corpus must not remove those bounds or
  make the first ranked result wait for a complete scan.
- Terminal content is read as decoded text. Nothing above `src/terminal/`
  handles escape sequences, so matching happens over presentation cells or
  scrollback rows, never over raw output.
- A picker's initial bare Space still closes it, and Space is a term separator
  once the query has text.

## Implementation notes

`FilePicker` and `ResourceFinder` are two ranking engines sharing one query,
switched by `FinderMode`. Merging their matches by score keeps both engines and
their separate scanners intact, which matters because the file scan is
asynchronous and unbounded in size while the resource list is small and rebuilt
on demand.

Content mode is the larger change. `FileEntry` carries an index into the
picker's table of distinct paths, and neither a pathless buffer nor a terminal
has a path, so a candidate's source has to become something wider than a path
before either can be ranked alongside file lines. The same widening is what
lets Enter dispatch by result kind.

## Documentation

`context/reference/helix-keymap-v1.md` is the register of record for these keys
and must be updated in the same change: the `/` and `S` rows in the search
table, the `Space / f`, `Space f`, `Space / g`, `Space / s`, and `Space / /`
rows in both the search table and the picker section, and the prose above them
recording why neither in-buffer flavour took a namespace spelling.
`context/reference/ui-vocabulary.md` covers picker and prompt vocabulary and
must describe the finder's two modes. `docs/user-guide.md` and `README.md`
describe the current search keys and need the same revision.
