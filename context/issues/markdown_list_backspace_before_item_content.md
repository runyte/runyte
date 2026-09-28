# Backspace after a list marker ignores text that follows the caret

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

## Observed behavior

`Backspace` at that caret deletes one ordinary character, the space of the
separator, leaving `3.to show the problem`. The special handling applies only
when the caret is at the end of a line that holds nothing but the marker.

## Expected behavior

`Backspace` directly after a list item's marker and separator behaves the same
whether or not text follows the caret:

1. The first press replaces `3. ` with three spaces, making the text a
   continuation line of item 2, with the caret still before `to`.
2. The second press removes that alignment, leaving `to show the problem` at
   the start of the line with the caret before it.

## Reproduction

1. Open a Markdown buffer containing the list above.
2. In Insert mode, place the caret between `longer ` and `to` and press
   `Enter`.
3. Press `Backspace`.
