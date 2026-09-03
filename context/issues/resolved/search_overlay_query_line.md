---
title: "Search, finder, and path overlays do not share one query line"
status: resolved
reported: 2026-09-03
resolved: 2026-09-03
commit: b163d50
---

## Resolution

Fixed in commit `b163d50`, "Give every query-owning overlay one query line".

`draw_snapshot_overlay` in `src/ui.rs` decided whether to reserve a query row
by asking whether `overlay.query` happened to hold text. That is a question
about the moment rather than about the surface, so an overlay opened empty had
no query line and every row under it moved down one as the first character
arrived. The decision now belongs to `OverlayInput`: a surface whose input is
`Filter` or `Text` keeps its line whether or not anything has been typed, and
a surface that owns no input — a completion showing its filter, the key-hint
menu showing a pending stroke — still draws only the text it has, and nothing
when it has none. `query_height` was already subtracted from the row capacity
and from the anchored overlays' height, so that accounting carried over
unchanged.

What the line reads while it is empty had to be published rather than guessed,
because the standalone renderers and the snapshot renderer would otherwise
invent their own invitations. `OverlaySnapshot` gained `query_placeholder`,
filled from `FilePickerKind::query_placeholder`, `FinderMode::query_placeholder`,
and `ListPicker::query_placeholder`, which are the single source both
renderers read. `query_line` in `src/ui.rs` builds the line once for
`draw_picker`, `draw_resource_finder`, `draw_list`, and
`draw_snapshot_overlay`.

`draw_list` dropped `type to filter` and `filter: <text>` from its title and
draws that line under it instead, so the title keeps the surface name, the
counts, and the action hints. Its rectangle also moved to the one the snapshot
renderer uses — `centered(editor_area, 80, 75, 28, 7)` rather than
`centered(editor_area, 86, 80, 24, 6)` — because the same rows sat on
different screen rows depending on which renderer was in front of them.

The completing prompts keep the interaction line, so they now publish no query
at all. `OverlayKind::CommandPalette` and `OverlayKind::ProgramHints` had
`OverlayInput::Text` and published the whole command line, which an attached
client drew inside the box while the same text stood on the interaction line
below it; both are `OverlayInput::None` with an empty query. Their path
assistance is a new kind, `OverlayKind::PathCompletion`, with
`OverlayPurpose::Context`, `OverlayInput::None`, and `OverlayLayout::Bottom`,
which is what lets a frontend tell assistance attached to the interaction line
from a choose-one overlay out of the snapshot alone.

`path_completion_area` anchors that kind to the bottom left of the editor
area and sizes it to the rows it holds, to the width they need, and to the
title and keys the border has to say, bounded by the editor and by twelve
rows. `draw_command_path_hints` is gone: `render_editor_frame` draws the
`PathCompletion` overlay from its snapshot, so the standalone list and the
attached one are the same list in the same corner by construction rather than
by two functions agreeing. An empty result keeps its `No matching paths` note,
which reaches the reader as the overlay's message; `OverlayPurpose::Context`
was added to the message colour mapping as muted, because assistance saying it
has nothing to offer is not reporting a failure.

`Space / p` is titled `Choose path for finder`. The palette's variant serves
every path-argument command, so one title names the one being completed —
`Choose path for :cd` — rather than a title per command. A row shows the
entry's own name, which `PathHint` now carries as `name`, and its detail
column keeps the resolved path only where it says something the name does not:
where `~` or a relative prefix was expanded, and not where the typed spelling
was already absolute and the resolved path would simply put the base back in
front of the row. `PathHint::value` is untouched, so what Tab inserts does not
change with what the row shows.

Protocol `VERSION` moved to 46. The new overlay kind and the placeholder are
both new on the wire: an older client has no case for the kind and would fail
to deserialize the frame, and an older host publishes the assistance as a
centred command palette whose query repeats the interaction line.
`OverlayKind::ALL` was also completed — `Path` and `PathActions` had been
missing from an inventory documented as exhaustive — alongside the new kind.

Tests:

- `the_finder_path_assistance_is_the_same_list_in_the_same_corner_in_both_renderers`,
  `the_finder_path_assistance_is_titled_for_the_finder_and_carries_no_query_of_its_own`,
  `a_hint_row_shows_the_entry_name_while_tab_still_completes_the_whole_spelling`,
  `a_palette_path_argument_is_titled_for_the_command_it_completes`, and
  `the_assistance_stays_bounded_when_a_directory_holds_far_more_than_it_can_show`
  in `tests/path_completion.rs`.
- `a_filterable_result_list_keeps_its_filter_on_a_query_line_that_is_always_there`,
  `the_two_renderers_agree_about_a_result_list_query_line`,
  `the_finder_keeps_its_query_line_before_and_after_its_first_character`, and
  `a_completing_prompt_publishes_no_query_of_its_own` in
  `tests/snapshot_boundary.rs`.
- `command_path_hints_distinguish_files_from_directories` and
  `the_choice_popup_is_wide_enough_for_its_longest_title` in `src/ui.rs`.

Known limitation: the command palette's own list of commands and the "open
with" program hints are still drawn by `draw_command_palette` and
`draw_program_hints` in a standalone editor, anchored to the bottom at up to
100 and 60 columns, while an attached client draws the same snapshots centred
at 80% by 75%. The report names the palette divergence but the scope it states
covers the finder, the filterable result lists, and the two path-completion
surfaces; only the duplicated query line was removed from those two here. The
plain result list likewise still pads its identifier column to forty cells in
the standalone renderer and not in the snapshot renderer, which is a column
layout rather than an arrangement of the surface.

## Report

The overlays that take a typed query show that query in three different
places, and one of them only shows it once it has text. A reader who moves
between the finder, a commit search, and a path prompt has to relearn where to
look, and rows move under the cursor as soon as the first character is typed.

### Observed behavior

- The finder (`draw_picker` in `src/ui.rs`) draws the query on the first line
  inside the border, and draws a muted placeholder — `> type to fuzzy-find` or
  `> type to fuzzy-search contents` — while the query is empty. The line is
  therefore always present and rows never move. This is the shape the other
  surfaces should take.
- Filterable result lists (`draw_list` in `src/ui.rs`, backed by
  `ListPicker`) put the filter **in the title**, as `type to filter` or
  `filter: <text>`, among the action hints. There is no query line under the
  title. `Space g /` (`:git-search-commits`) opens one of these, as do the
  buffer, terminal, stash, branch, and worktree lists.
- The shared snapshot renderer (`draw_snapshot_overlay` in `src/ui.rs`), which
  draws overlays for an attached client and for the snapshot-backed surfaces,
  reserves a query row only when `overlay.query` is non-empty. An overlay
  opened with an empty query has no query line, and every row shifts up by one
  the moment a character is typed.
- The path prompt behind `Space / p` (`PromptKind::FinderPath`) is not an
  overlay input at all. The typed value lives on the interaction line, labelled
  `find under path: `, and the rows above it are assistance rather than a
  choose-one request.
- **The two renderers draw that assistance completely differently.** In a
  standalone editor, `render_editor_frame` calls `draw_command_path_hints`,
  which anchors a bordered list to the bottom left of the editor area, sized
  to the number of hints up to 24 rows and up to 100 columns, titled
  `Paths · the finder is rooted here · ↑/↓ select · Tab complete`. In an
  attached persistent session, `render_attached_frame` has no such path: the
  prompt is published as an `OverlayKind::CommandPalette` overlay titled
  `Paths`, which falls to the default arm of `draw_snapshot_overlay` and is
  drawn `centered(editor_area, 80, 75, 28, 7)` — a box 80% of the editor's
  width and 75% of its height, in the middle of the screen, whatever the
  number of rows. Its title is `Paths · Enter accept · Esc cancel`, assembled
  from the overlay's actions rather than from the standalone string. The same
  prompt is a small list in one corner in one mode and a centred wall in the
  other.
- The typed path is published as that overlay's `query`, so in the attached
  renderer it is drawn as a `> <path>` line inside the box as soon as it has a
  character — while it also stands on the interaction line below. Each row then
  repeats it a third time: `PathHint::value` is the complete spelling that
  would replace the argument, and the row's detail column carries the resolved
  absolute path (`.agents/  directory · .agents`). One typed string, three
  places on screen.
- Because `draw_snapshot_overlay` reserves the query row only when the query is
  non-empty, that line appears on the first keystroke and pushes every row down
  by one.
- The same divergence covers the command palette itself, for the same reason:
  `draw_command_palette` anchors it to the bottom of the editor area at up to
  100 columns, while an attached client draws the identical
  `OverlayKind::CommandPalette` snapshot centred at 80% by 75%.
- Every colon command that takes a path argument — `:cd`, `:open`, `:explorer`,
  `:write`, `:vsplit`, `:hsplit`, `:file-picker-path`, `:session-attach`,
  `:session-stop` — reaches the palette's path-hint branch and behaves exactly
  as `Space / p` does. Typing `:cd ` in an attached session opens the same
  centred box over most of the editor. Its query line is worse there: the
  snapshot publishes the whole command line rather than the argument, so the
  box reads `> cd` while `:cd ` stands on the interaction line below it.

### Expected behavior

Every overlay that owns its own query — the finder in all three scopes and the
filterable result lists — has the same shape:

- the first line under the title is the query line;
- that line is present and legible when the query is empty, with a muted
  placeholder, so accepting rows never move as the query gains its first
  character;
- the query is not also carried in the title, which keeps the surface name,
  the counts, and the action hints.

The path surfaces keep the interaction line and are covered separately below.

Both renderers must agree: the standalone `draw_*` functions and
`draw_snapshot_overlay` are two readings of one surface, and an attached
client must not see a different arrangement from a standalone editor.

### Scope

In scope are the surfaces that draw an overlay above a typed query: the finder
in all three scopes, the filterable result lists including `Space g /`, and the
two path-completion surfaces, which are treated differently for the reason
given below.

Out of scope are the plain interaction-line prompts that have no overlay at
all — `s`, the in-buffer regular-expression search, and the workspace search
prompts under `Space /`. Those are labelled prompts on the interaction line
(`search: `, `search (regex): `, `workspace search…`) and are not changed
here.

### The path surfaces

`context/reference/ui-vocabulary.md` defines a **completing prompt** as an
interaction-line prompt whose rows are a completion of the value being typed
rather than a choose-one request, and states that "the prompt keeps the
interaction line and Enter still accepts what was typed rather than what is
selected". The palette's path arguments and the `Space / p` finder-path prompt
are named as the two of these.

That contract is worth keeping: it is why Tab completes and Enter takes the
typed text, and moving the input into an overlay would make the path surface
answer differently from the palette it shares its completion with. What is
wrong is that the assistance is drawn as though it owned the input, and that
the two renderers disagree about even that.

The expected shape is assistance attached to the line it completes, identical
in both renderers, and it applies to `Space / p` and to every colon command
that takes a path argument:

- the list is anchored immediately above the interaction line, at the left of
  the editor area, rather than centred;
- it is sized to the rows it holds — a few rows tall, wide enough for the
  entries — rather than to a percentage of the editor or to a fixed 100
  columns;
- it carries no query line of its own. The interaction line one row below it
  is the query, so the typed path appears once on screen. The overlay
  snapshot must stop publishing the typed value as its `query`, or the
  renderer must stop drawing a query for this kind, whichever keeps the
  snapshot honest about what the surface is;
- a row shows the entry's own name — the part completion would add — with the
  shared base implied by what is already typed, so the base is not repeated on
  every row. The detail column keeps whatever it says that the name does not;
- the title is `Choose path for finder` for `Space / p`. The palette's
  variant serves every path-argument command, so it is named for what those
  rows are rather than for one of them, and it says which command is being
  completed only if that can be done without a title per command.

Because the interaction line remains the query line, the rule in the previous
section applies to the overlays that own their input: the finder and the
filterable result lists.

The overlay kind is part of this. The finder-path prompt is published as
`OverlayKind::CommandPalette`, which is why it inherits the palette's centred
default geometry; `OverlayKind::Path` exists and is used for the read-only
`:path` popup. Whether the completing prompts take a kind of their own, or the
renderer grows a bottom-anchored case for them, is the fix's decision, but a
frontend must be able to tell "assistance attached to the interaction line"
from "a choose-one overlay" out of the snapshot alone.

### Constraints

- Reserving a query row costs a row of results. `draw_snapshot_overlay`
  already subtracts `query_height` from its row capacity; the same accounting
  has to hold for a query line that is now unconditional, including for the
  anchored completion, signature, and hover overlays that share the function
  and must **not** grow a query line they have no query for.
- `OverlaySnapshot` already carries `query`, `query_cursor`, and
  `OverlayInput`; the decision of whether to draw the line belongs to
  `OverlayInput` rather than to whether `query` happens to be empty.
- Titles are user-visible strings pinned by tests in `src/ui.rs` and
  `tests/snapshot_boundary.rs`.
- `PathHint::value` is the spelling that replaces the argument on accept.
  Showing a shorter row is a rendering decision; the value a row carries and
  what Tab inserts must not change with it.
- `context/reference/ui-vocabulary.md` describes the picker header counts and
  their pacing; the counts stay in the title.

### Regression coverage

Assert at the snapshot and rendered-frame boundary that each affected overlay
shows a query line while its query is empty and that its rows keep their
screen rows when the first character is typed. For the path surfaces, assert
that both renderers place the list in the same corner at the same size for
the same hints, that it is bounded in height and width, that the typed path
appears once rather than in a query line as well, that a row shows the entry
name rather than the full typed spelling while still completing to the same
value, and that the `Space / p` list is titled `Choose path for finder`.
`tests/path_completion.rs` and `tests/snapshot_boundary.rs` are the existing
homes for those assertions.
