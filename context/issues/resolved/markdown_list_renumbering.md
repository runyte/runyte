---
title: "Inserting or removing a numbered list item leaves later numbers stale"
status: resolved
reported: 2026-09-28
resolved: 2026-09-28
commit: 01736b9
---

## Resolution

Commit `01736b9` (`Renumber Markdown list items after inserting or removing
one`) added renumbering to `App::edit_newline` and `App::edit_backspace` in
`src/app/editing.rs`. Both built each caret's change from the item on its own
row and never looked at the rows below, so an item started by `Enter` took the
number after its predecessor while the sibling already holding that number
kept it.

Three edits now renumber: `Enter` starting an item after the caret's item,
`Enter` on an empty item (which removes its marker), and `Backspace` that
turns an item into a continuation. That last one covers the round trip the
report asks for, and the second means `Enter` `Enter` inside a list leaves a
blank line without changing any number.

A new `Renumbering` value records the markers siblings will carry, keyed by
pre-edit row. `shift` walks the rows after an anchor. Blank lines, deeper
continuation lines and nested items are passed over; anything indented no
deeper than the siblings ends the walk. A sibling is renumbered only while its
marker is the one that would have followed its predecessor's marker as
written, which is what keeps a list of repeated `1.` markers, or one with a
deliberate gap, unchanged from that point on. `list_marker` and
`following_marker` were factored out of `parse_list_item` and
`next_list_prefix` so the same successor rule serves continuation and
renumbering for decimal, letter and Roman markers. Bullets follow themselves,
so they produce no renumbering.

Several carets compose as if applied one at a time from the top. Carets are
visited in order; a run that reaches a later caret's item gives it its new
marker and stops, and that caret carries the run on from there, so no row is
walked twice. The inserted item of a caret whose own item was renumbered
follows the new number. A run that reaches a later anchor in Roman style hands
that style over as well, because `roman_style_before` stops at a blank line
that the run itself passes over. Renumbering is skipped entirely when a range
spans rows or two carets share a row. A sibling whose marker another caret's
change overlaps, or whose line break that change removes, stops the run
rather than be rewritten under it.

Coverage is in `src/app/tests/markdown_list_continuation.rs`:
`markdown_enter_renumbers_following_items_and_backspace_restores_them` walks
the reported example and its one-step undo;
`markdown_enter_at_the_end_of_an_item_renumbers_the_same_way`,
`markdown_enter_on_an_empty_item_renumbers_the_items_after_it`,
`markdown_renumbering_follows_every_ordered_marker_style`,
`markdown_backspace_renumbering_follows_letters_roman_numerals_and_tabs`,
`markdown_renumbering_keeps_numbers_that_were_not_in_sequence`,
`markdown_renumbering_passes_over_nested_items_continuations_and_blank_lines`,
`markdown_multicaret_enter_and_backspace_renumber_one_list_consistently`,
`markdown_renumbering_continues_past_other_carets_unless_they_edit_a_marker`,
`markdown_renumbering_hands_its_roman_style_to_a_later_caret` and
`markdown_renumbering_leaves_a_marker_joined_onto_the_line_above` cover marker
styles, CRLF, tabs, overflow, width changes and caret positions, nested lists,
and multi-caret composition and conflicts.

Known limitation: a marker that grows wider, such as `9.` becoming `10.`, is
not followed by re-indenting the item's nested content, so a child indented
for the narrower marker may stop being a child under CommonMark. Renumbering
also stops early, leaving the rest of the list as written, at a lazy
continuation line at the siblings' own indent, at a bare `3.` with no
separator, where tabs and spaces are mixed between siblings and their
continuations, and at a sibling joined onto the line above by another caret's
`Backspace`, even onto a blank line where it stays an item. A lone caret reads
a single-letter numeral after a blank line (`I.`, `V.`, `X.` and so on) as a
letter, as continuation already did, so `V.` there is not renumbered from `IV.`.

## Report

With `editor.smart_newline` enabled in a Markdown document, `Enter` inside a
numbered list starts an item numbered one past the current one, and
`Backspace` directly after a new item's marker removes it. Neither edit
changed the items that follow.

```text
1. First point
2. Second point a bit longer <Enter>to show the problem
3. Some later point
```

`Enter` at the marked position produced two items numbered `3`:

```text
1. First point
2. Second point a bit longer
3. to show the problem
3. Some later point
```

Expected: the following items are renumbered so that the list stays
sequential:

```text
1. First point
2. Second point a bit longer
3. to show the problem
4. Some later point
```

Removing the new marker again with `Backspace` restores the original numbers,
so `Enter` followed by `Backspace` leaves the numbering as it was. The same
holds at the end of a line, where `Enter` starts an empty item.

Only items that were numbered in sequence with the inserted or removed item
should change. A list written with the same number on every item, such as
`1.` repeated, or one with a deliberate gap, keeps those numbers. Nested items
and continuation lines between siblings are not renumbered.

Reproduction: open a Markdown buffer containing the list above, place the
caret between `longer ` and `to` in Insert mode, and press `Enter`.
