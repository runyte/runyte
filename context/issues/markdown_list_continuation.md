# Smart newline aligns under a list item but never starts the next one

With `editor.smart_newline` on, `Enter` at the end of a list item places the
new line under the item's first content character. `list_continuation_indent`
in `App::edit_newline` (`src/app/editing.rs`) produces that indent for
`-`, `*`, `+`, decimal, single-letter and uppercase Roman-numeral markers, in
every file type.

That suits an item that continues onto a second line. It gets in the way of
the more common case, a list of one-line items:

```text
1. First
2. Second
3. Third
```

After `Enter` at the end of `1. First`, the caret sits three columns in. The
user must press `Backspace` three times before typing `2.`, and must repeat
that for every item. A marker as wide as `10.` needs four presses.

## Expected behavior

In Markdown buffers, `Enter` continues the list, and `Backspace` turns a new
item into a continuation line.

| Caret position | Key | Result |
| --- | --- | --- |
| After the content of `1. First` | `Enter` | New line `2. `, caret after it |
| After the content of `- item` | `Enter` | New line `- `, caret after it |
| On an empty item: indent, marker, separator, nothing else | `Enter` | Marker removed; the line becomes empty (no indent), ending the list |
| Directly after an empty item's marker and separator | `Backspace` | Marker replaced by spaces of the same width: a continuation line aligned under the previous item's content |
| At the end of a whitespace-only continuation indent | `Backspace` | The whole alignment indent is removed in one press, back to the list item's own indentation |

The single-line list then needs no extra keys:
`First` `Enter` `Second` `Enter` `Third`. A wrapped item needs one extra key:
`Enter` `Backspace`, then the continuation text. `Enter` `Enter` leaves the
list.

Details:

- **Scope.** Marker continuation and the list-aware `Backspace` apply only
  in Markdown buffers: files whose language is Markdown, and the scratch
  buffer when `scratch_reads_as_markdown` holds. In other file types, list
  alignment keeps its current behavior, which is useful in YAML and in
  comments. Auto-typing `- ` or `2. ` into code or YAML would be wrong.
- **Setting.** All of this stays under `editor.smart_newline`. With it off,
  `Enter` keeps only the row's existing indentation, as now.
- **Rules by shape.** An item counts as empty because of its shape, not
  because Runyte inserted its marker. A marker typed by hand behaves the same
  way.
- **Numbering.** A decimal marker increments (`9.` → `10.`, keeping the
  same separator whitespace). A single letter advances within its case
  (`a.` → `b.`, `A.` → `B.`). A multi-letter uppercase Roman numeral
  increments as a numeral (`II.` → `III.`, `IV.` → `V.`). A single `I`, `V`,
  `X`, `L`, `C`, `D` or `M` is ambiguous. The existing detector treats a
  single character as a letter, so `I.` continues as `J.`. Keep that unless a
  preceding sibling item shows the list is Roman. After `z.`/`Z.` there is no
  next letter. Fall back to today's alignment in that case.
- **Bullets.** A bullet continues with the same character and separator.
  A task item `- [ ] text` or `- [x] text` continues as `- [ ] `, unchecked.
- **Caret mid-line.** `Enter` in the middle of an item's content moves the
  text after the caret onto the new item: `1. Fi|rst` becomes `1. Fi` and
  `2. rst`. `Enter` before or inside the marker is not a list continuation.
  It keeps today's behavior.
- **Renumbering.** Renumbering the items below a new or removed item is out
  of scope.
- **Nesting.** Nesting an item deeper with `Tab` on an empty item is out of
  scope.
- **Undo and multiple carets.** Each `Enter` or `Backspace` is one edit, as
  now. With several carets, each caret's result is derived from the pre-edit
  text, as `edit_newline` already guarantees.
- **Replace mode.** `Enter` in Replace mode is unaffected.

Undecided: whether the one-press `Backspace` over a continuation indent also
applies outside Markdown, where list alignment still happens. Decide while
fixing this. If it does, it must only fire on an indent that matches list
alignment, never on ordinary code indentation.

## Documentation and coverage

- Update the smart-newline paragraph in `docs/user-guide.md` and the
  `editor.smart_newline` description in `src/settings.rs` and
  `config.example.yaml` to describe list continuation in Markdown.
- The resolved records `smart_newline.md` and `smart_newlines.md` describe
  the alignment this builds on. Do not edit them unless their regression
  tests change.
- Tests: continuation for each marker kind (bullet, decimal including
  `9.` → `10.`, letter, uppercase Roman, task item); `Enter` on an empty item
  ends the list; `Backspace` after an empty marker gives the aligned
  continuation; `Backspace` on a continuation indent removes it in one press;
  `Enter` in the middle of an item; the same keys in a non-Markdown buffer
  keep today's alignment; `smart_newline: false` disables all of it; a
  multi-caret case.
