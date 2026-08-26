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

Known limitation: a click past the end of a line still places the caret on the
line's own end offset rather than its last character, which Normal-mode motion
never does. Dragging there therefore covers the whole line's text and no
further, since `operative_span` clamps to the row end, but the caret sits one
place beyond where a keyboard motion would leave it. That predates this fix
and is untouched by it.

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
