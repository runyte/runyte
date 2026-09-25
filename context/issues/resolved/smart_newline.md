---
title: "Enter did not align continuation lines under list item content"
status: resolved
reported: 2026-08-13
resolved: 2026-08-13
legacy_commit: b9ffb73
---

## Resolution

Commit b9ffb73 (`Align smart newlines for list continuations`) fixed the newline
indentation in `App::edit_newline`. That function previously copied only the
whitespace before the insertion point and optionally added one syntax-requested
tab-width level. As a result, it could preserve a line's ordinary indentation
but could not derive a hanging indent from a list marker, and long ordered
markers were aligned to the fixed syntax level rather than their content.

The pre-edit newline projection now recognizes `-`, `*`, and `+` bullets,
decimal markers, single-letter alphabetic markers, and Roman-numeral markers.
It replaces the marker with an equal-width space run while retaining the exact
leading and separator whitespace, so nested items and markers of different
lengths align under their first content character. The calculation remains
part of the existing pre-edit, single-transaction multicursor path. A typed
`editor.smart_newline` setting was added to YAML and the settings picker. It
was initially enabled by default; it is now disabled by default and can be
enabled to request the syntax- and list-aware behavior.

A later review found that accepting any case-insensitive run of Roman-numeral
letters also mistook words such as `civil.` and `mix.` for markers. The detector
therefore accepts multi-letter Roman markers only when they use uppercase
canonical Roman-numeral order; lowercase `i.` remains supported through the
single-letter alphabetic marker rule.

A later Markdown-specific change continues list markers on Enter and uses
Backspace to switch an empty item to its hanging indent. The original hanging
indent remains the behavior in non-Markdown buffers. The two alignment tests
below now disable scratch Markdown to exercise that boundary.

Tests covering the behavior are
`smart_newline_aligns_list_continuations_under_their_content`,
`smart_newline_keeps_following_continuation_lines_aligned`, and
`smart_newline_ignores_prose` in `src/app/tests/editing_and_buffers.rs`, plus
`smart_newline_is_default_off_and_configurable` in `src/config.rs`.
Markdown-specific continuation and Backspace are covered by
`src/app/tests/markdown_list_continuation.rs`.

## Report

Smart newlines were requested as an enabled-by-default feature configurable in
the editor configuration. Pressing Enter was expected to move the cursor to
the next line and choose its column from both the first character's position
on the preceding line and any list marker on that line.

Ordinary leading indentation was already preserved, but list continuation
indentation was not. Unordered list markers `-`, `*`, and `+` were expected to
leave the marker protruding while the continuation aligned with the item text.
Ordered markers also needed to support decimal numbers of varying length,
lowercase and uppercase letters, and Roman numerals, as in:

```text
1. This is the first line
   and this is the second line aligned so that the numbers stick out

10285. And if the item number is long, the second line
       is aligned accordingly.
       The next lines are also aligned.
```

Lettered lists included both forms:

```text
a. First
b. Second
```

```text
A. First
B. Second
```

Roman-numeral lists included:

```text
I. First
II. Second
```

The alignment also needed to respect nested indentation:

```text
1. First point
   second line
    - Nested point
      with second
      and third line
        + Even more nested
            a. And even more nested and ordered list starting with a letter
               with correct indendation of the second line.
```
