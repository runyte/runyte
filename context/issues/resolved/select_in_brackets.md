---
title: "Structural selections include a trailing character and delimiter objects fail in Markdown"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: 3694a76
---

## Resolution

Fixed in commit `3694a76`, "Fix structural selection bounds".

`App::expand_syntax_selection`, `App::transform_syntax_selection`, and
`App::select_syntax_object` installed half-open ranges returned by the syntax
layer but left the pane tagged with Runyte's inclusive selection semantics.
Yank, delete, change, indentation, and rendering could therefore include the
character at the exclusive end. `ExpansionHistory` also retained the range and
mode but not its coordinate semantics, so shrinking could not faithfully
restore the selection that existed before expansion. All syntax-produced
selections now carry half-open semantics, and expansion snapshots preserve and
restore the previous semantics as well as direction, primary range, and mode.

`DocumentSyntax::enclosing_delimiter` previously recognized only syntax nodes
whose first and last characters were a matching pair. That is correct for
source-language trees, but Markdown represents ordinary prose punctuation as
text rather than delimiter-shaped nodes. Markdown prose now has a balanced,
escape-aware fallback bounded to the smallest enclosing Markdown syntax node.
This deliberately departs from a purely Tree-sitter-node implementation only
where the Markdown grammar provides no structural pair; injected code keeps
using its own syntax tree, so brackets in code strings and comments do not
become prose matches.

A follow-up corrected the visible endpoint convention without changing those
exact syntax bounds. `App::select_syntax_object`, `App::select_delimiter`, and
`App::transform_syntax_selection` now translate the syntax layer's exclusive
end into Runyte's inclusive block cursor convention. Text objects and
parent/child/sibling selections therefore rest on the final character that
will be yanked, just as they do after `v e`; repeated structural navigation
and Vim visual operators translate that representation back to internal
half-open bounds before resolving or acting.

Covered by `syntax_namespace_selections_edit_their_exact_ranges` and
`delimiter_text_objects_work_in_markdown_prose` in
`tests/headless_editor.rs`, plus
`expansion_history_restores_direction_primary_and_mode` in
`src/structural_selection.rs`, and
`syntax_namespace_text_objects_end_on_the_last_included_character` plus
`visual_yank_includes_the_character_under_the_cursor` in `src/app.rs`. Run
with `cargo test --test headless_editor` and `cargo test structural_selection`.

Known limitation: delimiter text objects still require a supported
syntax-enabled buffer. Pathless plain-text buffers and unknown file types have
no syntax boundary in which to resolve them.

## Report

`Space x i` and `Space x a` behaved inconsistently.

With the cursor at the start of

```
This is a first example sentence.
```

`v e y` yanked `This`, selecting the whole word with the cursor on `s`. That
was correct.

With the cursor inside the parentheses of

```python
def fun(a, b, c):
```

`Space x i (` selected `a, b, c)` and left the cursor on `)`, so `y p`
produced `a, b, c)`. `Space x a (` (and `Space x a m`) yielded `(a, b, c):`.

`Space x i` should select only the text inside the matching brackets, and
`Space x a` only that text plus the brackets.

Separately, the two commands did not work in every file — they worked in Rust
but not in Markdown — which was unexpected for a tree-sitter-backed feature
and formed part of the report.
