---
title: "A long rendered-view label hides every name in the Navigator"
status: resolved
reported: 2026-09-27
resolved: 2026-09-28
commit: 1b0d86a
---

## Resolution

Commit `1b0d86a` (`Keep rendered page names visible in destination lists`)
corrected `App::buffer_destination_row`. It had found a generated buffer's TYPE
by taking its display name through the first `]`. A rendered Markdown page is
named `[rendered <file name>]`, so its file name became part of TYPE.
`App::destination_items` then widened that column for every row; the list
renderer had no room left to show any NAME at the reported width.

The row now recognizes the Markdown render's structural buffer identity and
uses `[rendered]` as TYPE, with the source name in NAME. The shared NAME
elision can shorten that name in the middle when space is limited. The
rendered page's pane title remains `[rendered <file name>]`; this is a list
column correction, not a rename of the buffer. This chooses the report's
short kind-label option instead of changing the column-width calculation.

Coverage is `rendered_markdown_keeps_its_file_name_in_the_destination_name_column`
in `src/app/tests/session_navigation.rs`, which checks both destination-list
scopes, and
`a_rendered_page_with_a_long_title_leaves_names_visible_in_destination_lists`
in `src/ui.rs`, which checks the Navigator and buffer list in both the local
and snapshot renderers.

## Report

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
[Session and destination navigation](../../../docs/user-guide.md#session-and-destination-navigation)
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
