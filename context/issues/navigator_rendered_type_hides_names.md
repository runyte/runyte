# A long rendered-view label hides every name in the Navigator

The Navigator, the buffer list and the terminal list draw `TYPE`, `NAME` and
`STATE` columns. A rendered Markdown view's TYPE label contains its file name,
`[rendered <file name>]`. When that name is long, the TYPE column grows to fit
it and the NAME column disappears for every row, not only for the rendered
view's row.

## Reproduction

In a 200-column terminal, create and open a Markdown file with a long name,
such as the prompt files coding agents write:

```text
claude-prompt-2d112e7d-f797-456b-8312-38d01f3e658b.md
```

Press `?` to render it and `?` again to return to the source. The rendered
view stays open as a buffer. Press `Space n`.

Observed (text of the Navigator's list pane):

```text
    TYPE                                                                  STATE
  * [terminal]
    [explorer]
    [about]                                                               [RO]
  * [file]
    [terminal]                                                            unread
    [file]                                                                [STALE]
    [rendered claude-prompt-2d112e7d-f797-456b-8312-38d01f3e658b.md]      [RO]
    [file]
```

No row shows its name, so the list cannot tell apart the two terminals or the
three files. Filtering by a name still matches, but the matching rows are
equally nameless.

Expected: NAME remains visible for every row. The rendered row's TYPE label is
shortened or kept to its kind, as a long NAME already is: the section
[Session and destination navigation](../../docs/user-guide.md#session-and-destination-navigation)
says a name too long for its row is shortened in the middle.

## Notes

After the rendered view is closed (`Space b c`), the same list draws normally,
and the file's own row shows a middle-shortened name:

```text
    [file]      …claude-prompt-0b9c1e44-7a52-4f0e-9d3b-5c8e2a61f7d0.md
```

The report does not decide whether the fix belongs in the column-width
calculation or in the rendered view's TYPE label. Other generated pages, such
as `[about]`, `[config]` and `[help]`, have short fixed labels and are not
affected.

Observed with Runyte 0.3.3 on Linux.
