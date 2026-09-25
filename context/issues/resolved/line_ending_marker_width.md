---
title: "Line-ending whitespace marker renders wider than one cell"
status: resolved
reported: 2026-09-25
resolved: 2026-09-25
commit: 8f9d1b3
---

## Resolution

Commit `8f9d1b3` (`Use one-cell line ending marker in common monospace fonts`)
fixed the glyph-coverage problem. `src/snapshot.rs` emitted U+21B5 `↵` for
real line endings. Although its declared display width was one cell, common
monospace fonts lacked the glyph and the terminal could draw a wider fallback.
The snapshot now emits U+00AC `¬`, a Latin-1 glyph with narrow East Asian
width, while keeping the existing CRLF coalescing, clipping, dim whitespace
style and caret role. The user guide, example config and keymap reference name
the new marker.

`whitespace_markers_preserve_cells_and_distinguish_real_line_endings` in
`src/snapshot.rs` covers LF/CRLF rendering, one-cell width, styling and the
unterminated final line. The focused test, formatting and Clippy passed.

Known limitation: the marker is fixed rather than configurable, as specified
in the report.

## Report

With whitespace rendering on (`Space p .`), a real LF or CRLF line ending is
drawn as `↵` (U+21B5 DOWNWARDS ARROW WITH CORNER LEFTWARDS). The run is
emitted in `src/snapshot.rs`. In some terminal and font combinations the
marker is visibly wider than the surrounding text cells. When the caret sits
on a line ending, for example after `A` in Insert mode, the caret block
covers about two cells instead of one.

Observed with JetBrainsMono Nerd Font as the terminal font:

```text
·····jjklasd↵      <- caret block on ↵ spans roughly two cells
·····fdafdsalkj↵
```

Space markers (`·`) and text on the same rows keep one-cell spacing.

## Diagnosis

The width reported by Runyte is correct. U+21B5 has East Asian Width `N`, so
the editor and the terminal both count it as one cell, and text after it does
not shift. The problem is glyph coverage. JetBrains Mono (and its Nerd Font
build) has no glyph for U+21B5, so the terminal draws it from a fallback
font. On the reporting system, fontconfig resolves U+21B5 to Adwaita Mono;
Noto Sans Mono also lacks it. The fallback glyph's advance and scale do not
match the primary font's cell. Terminals that let a glyph overflow into an
empty neighbouring cell draw it wider, and the cursor background follows the
glyph.

Glyph coverage checked with `fc-list` on the reporting system:

| Glyph | Code point | East Asian Width | JetBrainsMono Nerd Font | Noto Sans Mono |
| --- | --- | --- | --- | --- |
| `↵` | U+21B5 | N | missing | missing |
| `↲` | U+21B2 | N | missing | missing |
| `⏎` | U+23CE | N | present | missing |
| `¬` | U+00AC | Na | present | present |
| `¶` | U+00B6 | A | not checked | not checked |
| `$` | U+0024 | Na | present | present |

## Expected behavior

The line-ending marker occupies exactly one cell in common monospace fonts,
without needing font fallback. Replace `↵` with `¬` (U+00AC NOT SIGN):

- It is in Latin-1, so practically every monospace font includes it at the
  font's own cell width.
- Its East Asian Width is `Na`, so no terminal setting for ambiguous-width
  characters can make it two cells. Avoid `¶`, whose width is `A`.
- It is a common end-of-line marker in other editors, such as Vim
  `listchars` configurations and several IDEs. `$` would be read as buffer
  text more easily.

Constraints:

- The marker still takes one display cell and is only drawn when the final
  visual segment has room, as now. CRLF still produces one marker.
- Keep the marker's existing dim whitespace styling and caret behavior.
- Making the marker configurable is out of scope.

The tab marker `→` (U+2192) has East Asian Width `A`. It is present in
JetBrains Mono and Noto Sans Mono and was not reported as wide, so it is not
changed here. A terminal set to treat ambiguous-width characters as wide
would still draw it as two cells.

## Documentation and coverage

- Update `src/snapshot.rs`: the marker run and the tests that assert
  `"a·→ b↵"` and search for `'↵'`, plus any other test or fixture that
  contains the old glyph.
- Update `docs/user-guide.md`: the `Space p .` row in the editing key table
  and the "Layout and whitespace display" section both name `↵`.
- Update `config.example.yaml` and `context/reference/helix-keymap-v1.md`,
  which also name `↵`.
- Add or keep a test that the line-ending marker's display width is one cell.
