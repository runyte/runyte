---
title: "Markdown reflow can alter code after an invalid closing fence"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: f9587dc
---

## Resolution

Commit `f9587dc` (`fix(editing): preserve nested Markdown fences during reflow`)
changes `wrap::markdown_fence` to return the marker and its length. Reflow
previously tracked only the marker character, allowing a shorter inner fence
or a fence with trailing text to close a larger code block and exposing its
remaining contents to prose wrapping. Closure now requires the same marker,
at least the opening length, and whitespace alone after the marker run.
Code contents remain unchanged and ordinary prose resumes after valid closure.

`markdown_reflow_keeps_shorter_nested_fences_inside_code`,
`markdown_reflow_requires_a_bare_closing_fence`, and
`markdown_reflow_resumes_prose_after_a_longer_closing_fence` in
`src/wrap/tests/mod.rs` cover backticks and tildes, nesting, trailing content,
and valid longer closers. All 24 wrap tests pass.

## Report

Markdown reflow identifies an open fenced block by its marker character only.
A later line beginning with three matching markers closes the block even when
the opener had more markers, or when text follows the would-be closing marker.
Text still inside the fenced block is then treated as prose and can be joined
or wrapped by `Space p r`.

For reproduction, select this Markdown source and reflow at a width shorter
than its long line:

`````text
````markdown
```rust
let first_statement = 1; let second_statement = 2;
```
````
`````

The inner three-backtick fence must remain content of the outer four-backtick
block. Reflow must preserve every line inside the outer block. A closing fence
must use the opening character, contain at least as many markers as the opener,
and have no non-whitespace text after its markers. The same rules apply to
tilde fences. A longer valid closing fence must close the block so following
prose can be reflowed.

`wrap::reflow_lf` currently toggles its fence state using `Option<char>` from
`markdown_fence`, which discards the run length and does not distinguish an
opening info string from a valid closing fence. The rendered-page parser in
`markdown.rs` already distinguishes fence lengths; this report concerns the
editing transform in `wrap.rs`.
