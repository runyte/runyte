---
title: "Mouse selection dropped the last character of the dragged span"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: 9631ecc
---

## Resolution

Commit `9631ecc` (`Cover the character a pointer drag ends on`) fixed the
coordinate model the pointer installs. Both pointer paths in
`App::handle_pointer_repeated` — the left press and the left drag — built
`Range::new(anchor, offset)` from the pressed cell and the cell under the
pointer, then marked the pane `SelectionSemantics::HalfOpen` whenever the two
differed. A half-open span stops before its end, so the character the drag
ended on was neither highlighted nor part of the yanked text, in both
directions. A drag ending one cell further right would have covered it, but
that cell is the next character, so no drag could ever select a word by its
own last letter.

A pointer names a character, not the boundary before it, so the span it
describes covers the pressed cell and the cell under the pointer inclusively.
That is Runyte's own range model, the one `operative_span` reads with `to + 1`,
which is why `v e y` copied the whole word while the mouse did not. The two
call sites now share `App::pointer_selection`, which builds that inclusive
range and marks the pane with the semantics the active grammar edits in:
Runyte keeps the range as it stands, and the Vim grammar converts it through
the existing `App::vim_inclusive_to_half_open` so its selection is still
stored one past its last covered character. A press that selects nothing is
still a bare caret in either grammar, so clicking has not become a one
character selection.

Shift-click extension reads its anchor through the new `App::pointer_anchor`
for the same reason. An inclusive range is anchored on a character it covers
whichever way it runs, but a half-open one — a Vim selection, or a delimiter
text object under either grammar — keeps its anchor one past the covered
character when it runs backward, so that anchor is converted back to the cell
the pointer works in before the new range is built.

`docs/user-guide.md` now states the rule with the rest of the pointer
behavior.

Covered in `src/app/tests/presentation_and_settings.rs` by
`a_pointer_drag_selects_through_the_character_it_ends_on`, which drags over a
word in both directions and yanks it, and asserts that a press that moves
nowhere stays a Normal-mode caret; and by
`a_pointer_drag_under_the_vim_grammar_covers_the_same_characters`, which
checks the same span under the Vim grammar's half-open coordinates.

A later commit finished the same coordinate mismatch at the other end of a
line. `App::pointer_offset` clamped a press to `line_len`, the offset of the
row's line break, so clicking the blank area past a line left the caret one
place beyond where any keyboard motion can put it — Runyte clamps a Normal
caret to the last character of its row, and only an Insert caret may sit
after it. It now takes an `insert` flag and finishes through
`Buffer::clamp_offset`, the same clamp motion uses: a press carries the
pane's own Insert state, and a Shift-click or a drag passes `false`, because
both are building a selection whose head addresses a character whatever mode
the pointer started in. Clicking past a line in Insert mode still appends to
it.

`pointer_click_drag_wheel_and_resize_use_the_prepared_projection` in
`src/app/tests/presentation_and_settings.rs` changed with that commit: its
drag ends past the end of the row, so its head is now the row's last
character rather than the line break's offset. The yanked text was already
the same either way, because `operative_span` clamps to the row end.
`a_press_past_the_end_of_a_line_lands_where_that_mode_lets_a_caret_sit`, in
the same file, covers the rule directly across a normal row, an empty row,
and an Insert-mode press that appends.

## Report

Mouse selection skipped the last letter of a selected word.

Example string:

```
Testtest
```

String copied with `x y`:

```
Testtest
```

String copied with `v e y`:

```
Testtest
```

String copied with mouse forward selection:

```
Testtes
```

String copied with mouse backward selection:

```
Testtes
```

Expected behavior: text selected with a mouse from the first to the last
letter should be copied entirely, just as with `v`-based selection.
