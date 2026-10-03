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
