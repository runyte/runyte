# Benchmark lacks a programming-language fixture every editor parses

The startup benchmark in `benchmarks/` measures Runyte alongside Neovim and
Helix. Its fixtures are `small.rs`, `small.txt`, `medium.rs`, `large.rs`,
`large.txt`, `large.md`, and `minified.json`.

Only `large.md` compares the three editors on equal terms, because it is the
only fixture all three parse with tree-sitter. The `.txt` fixtures carry no
language for any editor and compare reading and drawing alone. The `.rs`
fixtures do not compare editors: Neovim has no Rust parser, so it highlights
those documents with regular expressions over the visible window while Helix and
Runyte build a tree over the whole document.

Markdown is therefore the only cross-editor evidence the benchmark produces, and
it is markup rather than source code. Markdown is also two grammars — block and
inline — driven through injections, which is not representative of how a
single-grammar programming language parses. The benchmark has no row that
answers how the three editors compare on code.

## What each editor parses

Neovim bundles seven parsers in `/usr/lib64/nvim/parser/`: `c`, `lua`,
`markdown`, `markdown_inline`, `query`, `vim`, and `vimdoc`. Bundling a parser
is not the same as using it. Only four runtime ftplugins call
`vim.treesitter.start()` — `help.lua`, `lua.lua`, `markdown.lua`, and
`query.lua` — so every other filetype falls back to regular-expression syntax
highlighting. Opening a C file reports `vim.treesitter.highlighter.active` as
absent and `&syntax` as `c`, despite `c.so` being present.

Of the four filetypes Neovim highlights with tree-sitter by default, `help` and
`query` are Neovim's own formats and `markdown` is already covered. **Lua is the
only general-purpose programming language in that set.**

Helix has a Lua grammar and query set installed at
`runtime/grammars/lua.so` and `runtime/queries/lua/`, covering highlights,
injections, locals, folds, indents, and textobjects.

Runyte has no Lua grammar.

## Expected behavior

A `large.lua` fixture is added to `benchmarks/fixtures.py` and to the `FIXTURES`
tuple, generated deterministically from the existing `SEED` in the same shape as
the other generators, with its own line-count constant beside `MARKDOWN_LINES`.
All three editors parse it with tree-sitter, making it the benchmark's first row
that compares the editors on source code.

The fixture table in `benchmarks/README.md` gains a row describing what it
isolates, and the section naming which rows support a cross-editor claim lists
`large.lua` alongside `large.md`.

A result set including the new fixture is recorded in
`context/reference/startup-performance.md`.

## Blocked by

`context/issues/additional_language_grammars.md`, which covers adding Lua among
seven other statically linked grammars. Until Runyte registers Lua, a `large.lua`
fixture would produce the same asymmetry the `.rs` rows already have, with the
editor lacking the grammar reading the document as plain text. This issue should
not be started before that one lands.

## Constraints

The fixture must not contain constructs that inject a second grammar, for the
reason recorded against `large.md`: each editor injects only the languages it
actually has, so an injecting construct measures the editors' differing grammar
inventories rather than their Lua parsing. For Lua this means avoiding long
strings that an injection query might treat as embedded content; the
`injections.scm` shipped with each editor's Lua grammar should be read before
choosing what the generator emits, since the three do not necessarily inject on
the same patterns.

Size should be chosen so parsing dominates fixed startup cost while staying well
below the five-second `PARSE_TIMEOUT`. `large.md` at 30,000 lines and 818 KB
produces a clear signal across all three, and is a reasonable starting point.

Run-to-run variance on the larger fixtures was about ten percent across two full
sessions at median of five runs, so the fixture has to produce differences larger
than that to be worth recording.

## Validation

Before any number is recorded, confirm each editor actually parses the fixture
rather than falling back. For Neovim, the check documented in
`benchmarks/README.md`:

```sh
nvim --headless FIXTURE \
  -c 'lua local b=vim.api.nvim_get_current_buf()
      print(vim.treesitter.highlighter.active[b] ~= nil, vim.bo.syntax)' -c q
```

`true` with an empty `syntax` is a tree-sitter parse; `false` with a syntax name
is the regular-expression fallback. For Helix and Runyte, confirm the grammar and
its queries are present in the build under test.

## Alternative considered

Neovim can be forced to parse a filetype it does not enable by default:
`nvim -c 'lua vim.treesitter.start()'` on a C file reports the highlighter as
active, which would make a C fixture comparable without waiting for Lua. This was
not chosen because the benchmark measures editors as shipped with an empty
`XDG_CONFIG_HOME`, and forcing the parser measures a configured Neovim instead.

That configuration is arguably more representative, since a typical Neovim
installation adds parsers for the languages in use, and the bare-editor rows
therefore report less work than a real installation performs. Whether to add a
separate forced-tree-sitter Neovim column is undecided and independent of this
issue; if it is added it should be a distinct column rather than a replacement
for the bare one.
