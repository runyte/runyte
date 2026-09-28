---
title: "Backspace after a list marker ignores text that follows the caret"
status: resolved
reported: 2026-09-28
resolved: 2026-09-28
commit: 682ab36
---

## Resolution

Commit `682ab36` (`Unwind a list marker on Backspace when text follows the
caret`) changed `App::edit_backspace` in `src/app/editing.rs`. Both Markdown
Backspace transitions were nested under a check that the caret sat at the end
of its line, and the marker transition also required the item to have no
content. An item split by `Enter` mid-line has its text after the caret, so it
never reached either transition and fell through to an ordinary one-character
deletion of the separator.

The marker transition now fires whenever the caret's character column equals
the item's content start, the point after indentation, marker, separator and
any task checkbox. `content_start` is a byte index into the line, but every
byte before it is ASCII by construction of `parse_list_item`, so its character
count is the comparable column. The pending-alignment transition now requires
only that the text before the caret be whitespace; it is still bound to the
buffer, pane, revision and caret position the first press recorded, so no
other indentation can disappear in one press. The whitespace-only fallback
scan that recovers an alignment without a pending record is unchanged and
still applies only at the end of a line.

The broadened condition applies to any list item, not only one just created by
`Enter`: Backspace directly after the marker of an existing item also turns it
into a continuation of the previous item. That matches the behavior an empty
item already had.

Coverage is in `src/app/tests/markdown_list_continuation.rs`:
`markdown_backspace_after_a_marker_unwinds_the_same_way_before_text` walks the
reported example through `Enter`, both Backspace transitions, and a third
Backspace that joins the lines back to the original text;
`markdown_backspace_at_an_existing_item_content_start_uses_character_columns`
covers multibyte content, tab separators, task items, and a wide marker after
a multibyte row, plus carets one column off the content start, inside a wide
separator, and on non-list text, which keep ordinary Backspace.

Known limitation: after the first press, the item's text remains a
continuation of the previous item, but the numbers of the items after it are
not adjusted; that is tracked separately in
`context/issues/markdown_list_renumbering.md`.

## Report

With `editor.smart_newline` enabled in a Markdown document, `Enter` at the end
of a list item starts the next item, and `Backspace` then turns that new item
into a continuation line in two steps:

1. The first `Backspace` replaces the marker with spaces of the same width.
2. The second `Backspace` removes that alignment in one press, leaving the
   caret at the start of the line.

`Enter` in the middle of an item also starts the next item and moves the text
after the caret onto it:

```text
1. First point
2. Second point a bit longer <Enter>to show the problem
3. Some later point
```

becomes

```text
1. First point
2. Second point a bit longer
3. |to show the problem
3. Some later point
```

with the caret (`|`) directly after the new `3. ` marker.

Observed: `Backspace` at that caret deleted one ordinary character, the space
of the separator, leaving `3.to show the problem`. The special handling
applied only when the caret was at the end of a line that held nothing but
the marker.

Expected: `Backspace` directly after a list item's marker and separator
behaves the same whether or not text follows the caret. The first press
replaces `3. ` with three spaces, making the text a continuation line of item
2 with the caret still before `to`; the second press removes that alignment,
leaving `to show the problem` at the start of the line with the caret before
it.

Reproduction: open a Markdown buffer containing the list above, place the
caret between `longer ` and `to` in Insert mode, press `Enter`, then
`Backspace`.
