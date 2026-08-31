---
title: "One search entry point did not cover files, buffers, and terminals"
status: resolved
reported: 2026-08-31
resolved: 2026-08-31
commit: c9a04df
---

## Resolution

Commit `c9a04df` (`Unify project search across editor resources`) made
`open-file-picker` one project finder with name and content modes. The finder
now merges asynchronous file matches with open-buffer and terminal matches,
uses authoritative in-memory text for open buffers, suppresses duplicate
file-backed buffer results, and preserves its query when Tab changes modes.
Its result target identifies files, buffer locations, and terminal locations,
so activation can open or focus the resource and reveal a matched content row.

`App::apply_terminal_output` had removed a terminal session as soon as its
child exited, which made its bounded decoded screen and scrollback unavailable
to search. It now removes the terminal only from panes and retains the exited
session until an explicit close. Focusing an exited terminal enters review,
and persistent-session idle retirement treats retained terminals as state that
must not be discarded.

The keymap registry now binds `/` and `Space / /` to the name-mode finder, `S`
to buffer regular-expression search, and `Space / S` to workspace
regular-expression search. The old `Space / f`, `Space f`, and `Space / g`
bindings were removed without removing their command-palette commands. Help,
key hints, the manual, user documentation, and both keymap and UI reference
records were updated from that registry-backed behavior.

Review tightened the completed boundary in five places. Name terms retain
their original case so the shared fuzzy matcher keeps smart-case behavior;
`Ctrl-s` and `Ctrl-v` still split-open selected file results; live buffer and
terminal content is visited in cancellable 128-row event-loop slices instead
of being materialized and scanned in one input pass; filesystem loading,
failure, skipped-file, and result-limit state reaches both semantic snapshots
and the terminal UI; and the retained keymap record describes `S` consistently.
Terminal output marks that terminal dirty without replacing a pending scan
cursor, then refreshes only its rows after the current corpus pass, so a busy
child cannot starve later buffer or terminal sources.

A second review tightened incremental behavior and presentation. Newly ranked
results may replace the automatic selection until explicit navigation claims
it; afterwards refreshes preserve the selected target. Disk and live content
hits share the one `CONTENT_ENTRY_LIMIT` admission budget, and live previews
are produced only for the selected row instead of being duplicated on every
hit. Filesystem failures remain visible without replacing valid buffer or
terminal rows, while file-only `Ctrl-s` and `Ctrl-v` actions are advertised
only for file targets. Content matches now carry character emphasis for the
detail column through both the core and protocol snapshots, so matching text
in buffer and terminal rows is visibly accented in standalone and attached
frontends.

A third review found that terminal invalidation still had two edge cases and
that cooperative row scanning was followed by non-cooperative ranking. Output
received after an idle scan now starts a terminal-only refresh, preserving
buffer results and a target claimed by navigation. Dirty terminal rows are
removed and their source is rescheduled even if another source reached the
shared limit, so stale output cannot survive a capped scan. Each 128-row batch
is now ranked once and linearly merged into the already-sorted results rather
than re-sorting every accumulated hit; terminal removal remaps retained
matches without rescoring them.

Coverage includes:

- `project_finder_switches_name_and_content_modes_without_losing_its_query`,
  `project_finder_keeps_file_split_activation`,
  `project_finder_snapshot_reports_filesystem_scan_failure`,
  `project_finder_content_reaches_and_activates_a_pathless_buffer`, and
  `project_finder_indexes_terminal_names_and_content_and_reveals_the_matching_row`
  in `src/app/tests/search_and_pickers.rs`;
- `pathless_buffer_content_is_scanned_in_bounded_slices`,
  `terminal_output_restarts_a_bounded_incremental_finder_scan`, and
  `repeated_terminal_output_does_not_starve_later_buffer_content` in
  `src/app/tests/search_and_pickers.rs`;
- `terminal_output_after_a_complete_scan_refreshes_only_that_terminal` and
  `dirty_terminal_rows_are_invalidated_when_another_source_reaches_the_limit`
  in `src/app/tests/search_and_pickers.rs`;
- finder incremental-ordering, selection, smart-case, literal content-term,
  and soft type-hint tests in `src/finder.rs`;
- `disk_hits_use_only_the_unified_content_budget_left_by_resources` in
  `src/app/tests/search_and_pickers.rs`;
- `resource_finder_highlights_matching_buffer_content`,
  `a_selected_row_keeps_match_emphasis_in_its_detail_column`, and
  `resource_finder_renders_buffer_preview_and_narrow_fallback` in `src/ui.rs`,
  together with the overlay-row wire round trip in `src/protocol/frame.rs`;
- `incrementally_ranking_a_full_live_content_budget_stays_bounded` in
  `tests/performance.rs`, run serially in the release performance job;
- `finder_and_workspace_search_are_global_in_every_buffer_scope` in
  `tests/keymap.rs` and the registry-derived hint assertions in
  `tests/key_hints.rs`;
- `exiting_the_last_terminal_reveals_its_buffer_without_quitting_runyte` and
  `exiting_a_terminal_preserves_its_pane_when_another_pane_exists` in
  `tests/terminal.rs`.

## Report

Project-wide search was spread across several bindings with different corpora,
and no single entry point covered everything the editor held open.

| Key | Command | Corpus |
| --- | --- | --- |
| `s` | `search` | the current buffer, escaped literal, ignoring case |
| `/` | `search-regex` | the current buffer, as a regular expression |
| `Space / s` | `global-search` | workspace files and authoritative unsaved buffer text |
| `Space / /` | `global-search-regex` | the same, as a regular expression |
| `Space / g` | `open-fuzzy-grep` | file content lines under the project root, preferring unsaved buffer text |
| `Space / f`, `Space f` | `open-file-picker` | file paths; Tab switches to a combined open-buffer and terminal mode that matches buffer names and paths and terminal names, program and title, stable ID, and directories |

Terminal output was not searchable by content anywhere in the editor. Buffer
content was searchable only through the file that backed a buffer, so a
generated or pathless buffer's text was reachable by name but not by what it
contained.

The name search that spanned files, buffers, and terminals was reached by a
three-key sequence or its `Space f` alias, and the content search that spanned
the project was a different sequence over a narrower corpus.

### Expected behavior

One finder searches anything the editor can reach, in two modes that Tab moves
between without clearing the query.

Its **name mode** ranks files under the project root, open buffers, and terminals
in one merged list. It opens in this mode. Files join the corpus the resource
mode already matched: buffer structural names and paths, terminal names,
program and title, stable ID, and current and initial directories, with
absolute, project-relative, `~/`-prefixed, and basename path spellings. The
soft type preference that ranks `terminal`, `term`, or `buffer` first rather
than filtering extends to files. The former file and resource modes stop being
separate stops, because Tab carries the mode switch.

Its **content mode** ranks content lines from those same three sources: file
content lines under the project root, buffer text including buffers with no
file, and terminal output. Tab switches to it from name mode and back, and the
query survives the switch, as it did across the finder's former two modes.

Enter activates the selected result according to its kind: a file opens, a
buffer becomes current, and a terminal is focused. A content result
additionally reveals the matched line, which for a terminal means scrolling
its scrollback to that row.

### Expected bindings

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
would contradict the prefix's stated meaning. Bare `S` was unbound: it had
carried the case-sensitive search flavour until the `Space /` rewiring and was
retired when `(?-i)` replaced it. The regular-expression flavour displaced
from `/` therefore lands on a free key that already read as a search variant.
In-buffer search keeps both flavours: `s` for the literal one it already had,
and `S` for the regular expression.

The command identities do not change, to avoid renaming across the palette,
help, key hints, and the tests that pin the inventory. Their palette
descriptions do: `open-file-picker` describes the finder rather than a file
picker, and `open-fuzzy-grep` describes file, buffer, and terminal contents. A
rename remains available if the retained wording proves misleading in the
palette.

### Result presentation

The finder is a live fuzzy picker in both modes, with the existing ranking,
preview, and `Ctrl-t` preview toggle. A content result emphasizes the matching
characters in the displayed buffer or terminal line, including while that row
is selected.

`global-search` and `global-search-regex` keep their retained `[workspace
search]` special buffer, with registry-backed Enter on a result jumping to its
typed source range, and the clean result remaining available while it is among
the eight most recently active special buffers. Fuzzy ranking stays ephemeral
and an exact pattern keeps producing a durable, reviewable list; the two
surfaces answer different questions and both remain.

### Terminal and buffer content corpus

Terminal content search reads the decoded text of every open terminal session,
live or exited: its scrollback, bounded at `SCROLLBACK_LIMIT` rows per session
by `src/terminal/grid.rs`, together with the current screen. An exited session
keeps its last screen readable until it is closed, so it stays searchable on
the same terms as a live one and needs no separate rule. No further bound is
introduced: the per-session scrollback limit, the workspace cell budget, and
the picker's own candidate budget already bound the corpus.

Buffer content search reads authoritative in-memory text for every open buffer,
including buffers with no path.

### Constraints

- Key dispatch, help, and key hints continue to read from the one keymap
  registry in `src/keymap.rs`.
- A file that is also an open buffer produces one result, not two, and the
  authoritative unsaved buffer text wins over what is on disk, as the existing
  content picker already arranged.
- Content search stays asynchronous and bounded. The report described the
  current picker as bounding candidates at 10,000 lines and skipping files
  over 4 MiB, while workspace search has its own file-size limit; widening the
  corpus does not remove those bounds or make the first ranked result wait for
  a complete scan.
- Terminal content is read as decoded text. Nothing above `src/terminal/`
  handles escape sequences, so matching happens over presentation cells or
  scrollback rows, never over raw output.
- A picker's initial bare Space still closes it, and Space is a term separator
  once the query has text.

### Implementation notes

`FilePicker` and `ResourceFinder` were two ranking engines sharing one query,
switched by `FinderMode`. Merging their matches by score keeps both engines and
their separate scanners intact, which matters because the file scan is
asynchronous and unbounded in size while the resource list is small and
rebuilt on demand.

Content mode was the larger change. `FileEntry` carried an index into the
picker's table of distinct paths, and neither a pathless buffer nor a terminal
has a path, so a candidate's source had to become something wider than a path
before either could be ranked alongside file lines. The same widening lets
Enter dispatch by result kind.

### Documentation

`context/reference/helix-keymap-v1.md` is the register of record for these keys
and needed updates to the `/` and `S` rows in the search table, the
`Space / f`, `Space f`, `Space / g`, `Space / s`, and `Space / /` rows in both
the search table and picker section, and the prose recording why neither
in-buffer flavour took a namespace spelling. `context/reference/ui-vocabulary.md`
covers picker and prompt vocabulary and needed to describe the finder's two
modes. `docs/user-guide.md` and `README.md` needed the same revision.
