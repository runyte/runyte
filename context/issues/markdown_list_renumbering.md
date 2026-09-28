# Inserting or removing a numbered list item leaves later numbers stale

With `editor.smart_newline` enabled in a Markdown document, `Enter` inside a
numbered list starts an item numbered one past the current one, and
`Backspace` directly after a new item's marker removes it. Neither edit
changes the items that follow.

```text
1. First point
2. Second point a bit longer <Enter>to show the problem
3. Some later point
```

## Observed behavior

`Enter` at the marked position produces two items numbered `3`:

```text
1. First point
2. Second point a bit longer
3. to show the problem
3. Some later point
```

## Expected behavior

The following items are renumbered so that the list stays sequential:

```text
1. First point
2. Second point a bit longer
3. to show the problem
4. Some later point
```

Removing the new marker again with `Backspace` restores the original numbers,
so `Enter` followed by `Backspace` leaves the numbering as it was. The same
holds at the end of a line, where `Enter` starts an empty item.

## Constraints

Only items that were numbered in sequence with the inserted or removed item
should change. A list written with the same number on every item, such as
`1.` repeated, or one with a deliberate gap, keeps those numbers. Nested items
and continuation lines between siblings are not renumbered.

## Reproduction

1. Open a Markdown buffer containing the list above.
2. In Insert mode, place the caret between `longer ` and `to` and press
   `Enter`.
