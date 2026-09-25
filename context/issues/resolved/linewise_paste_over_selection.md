---
title: "Pasting a linewise register over a partial selection replaces whole lines"
status: resolved
reported: 2026-09-25
resolved: 2026-09-25
commit: f502a87
---

## Resolution

Commit `f502a87` (`Respect selection shape when pasting linewise text`) fixed
the replacement span in `App::paste_register`. It previously widened every
selection to whole rows when the register was linewise. The paste now uses
the selection's provenance: a characterwise range replaces exactly its text
and omits only the register's final LF or CRLF, while a transient `x`/`X` or
Vim line selection replaces complete selected rows. A characterwise range
covering a line's text without its terminator remains characterwise. Bare
carets still insert linewise content as whole lines, and `P` still does not
replace.

The replacement gate also uses line-selection provenance when a one-character
or empty row produces a point-shaped range. It derives the row from raw
selection endpoints so a final empty row does not consume the preceding
line's terminator. Register and clipboard paste share this rule, and the
replacement remains selected without consuming the register.

Coverage is in `src/app/tests/editing.rs` for characterwise, linewise,
multi-range, LF/CRLF and point-shaped row selections, and in
`src/app/tests/editing_and_buffers.rs` for clipboard paste. All 60 paste
tests passed, as did formatting.

## Report

`p` on a range that holds text replaces that text with the register.
When the register is linewise, the one written by `x y`, `Y`, or `d` after
`x`/`X`, `replaced_span` in `src/app/editing.rs` widens the replaced span to
every whole line the selection touches. A selection inside one line therefore
loses the whole line instead of only the selected characters.

Reproduction:

1. On some line, press `x y` to yank the whole line, `copied line`.
2. On another line, select only the capital letters:

   ```text
   example line WITH THESE CAPITAL LETTERS selected
   ```

3. Press `p`.

Observed: the entire `example line … selected` line is replaced by
`copied line`.

Expected: only `WITH THESE CAPITAL LETTERS` is replaced:

```text
example line copied line selected
```

The same text yanked characterwise with `v … y` already replaces only the
selected characters. The current whole-line behavior is deliberate and
documented: see the `p`, `P` row of `context/reference/helix-keymap-v1.md`
("a linewise register replaces the whole lines the range touched") and the
paragraph after the editing key table in `docs/user-guide.md`. This issue
changes that rule.

## Expected behavior

A register's linewise flag decides how it is inserted at a bare caret. It
does not widen a selection the user made.

- **Bare caret, `p`/`P`:** unchanged. A linewise register pastes as whole
  lines below/above the caret's line.
- **Characterwise range, linewise register, `p`:** replace exactly the span
  `d` and `c` would act on, as with a characterwise register. Remove the
  register's final line terminator (`\n` or `\r\n`) from the inserted text,
  so a single yanked line does not split the target line. A register of
  several lines keeps its inner line breaks; only the last terminator is
  dropped.
- **Line selection, linewise register, `p`:** a transient `x`/`X` line
  selection or a Vim linewise selection keeps today's behavior. The whole
  selected lines are replaced by the register's lines, terminators included.
  Line replaces line.
- **`P`:** unchanged. It never replaces.
- **After the paste:** the replacement stays selected, and the register is
  not consumed, as now. A linewise register therefore still reads as
  linewise at the next bare-caret `p`.
- **Multiple ranges:** a multi-selection, for example every search match,
  replaces each range with the trimmed text. Ranges that previously
  overlapped only because of linewise widening no longer overlap, so each
  keeps its own replacement.
- **Clipboard:** `Space c p` follows the same rule from the system clipboard.

Undecided: whether a characterwise range that covers a whole line's text
exactly, without its terminator, counts as a line selection. The rule above
treats it as characterwise, because only `x`/`X` and Vim linewise selections
carry line provenance. Confirm this while fixing the issue.

## Documentation and coverage

- Update the `p`, `P` row of `context/reference/helix-keymap-v1.md` and the
  paste paragraph in `docs/user-guide.md` to state the new rule. Keep the
  existing notes about bare carets and `P`.
- Resolved records `copy_line.md` and `x_d_then_paste.md` describe how
  linewise registers are produced and pasted at a caret. That behavior is
  unchanged, so they need no edit unless their regression tests change.
- Tests: a linewise register pasted over a partial single-line selection, a
  multi-line linewise register over a partial selection (inner breaks kept,
  final terminator dropped, CRLF included), the same register over an `x`
  line selection (whole lines replaced), the same register at a bare caret
  (whole-line paste, unchanged), and a multi-range replacement.
