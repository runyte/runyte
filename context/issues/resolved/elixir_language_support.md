---
title: "Elixir files lack language-specific syntax highlighting"
status: resolved
reported: 2026-09-23
resolved: 2026-09-23
commit: 9ba781f
---

## Resolution

Commit `9ba781f` (Add Elixir Tree-sitter highlighting) registered the pinned
`tree-sitter-elixir 0.3.5` grammar in `BUILTIN_LANGUAGES`. Previously,
`src/syntax/grammars.rs` had no Elixir definition, so path detection and
Markdown fence lookup could not select an Elixir parser. The packaged query's
`@module` capture was unmapped in Runyte and its atom capture fell back to
string colouring. A small Elixir query fragment maps aliases and atoms to
existing namespace and constant scopes after the packaged highlights. The
existing background parser and bounded Markdown injection path need no Elixir
specific scheduling or runtime service.

The regression tests in `tests/syntax.rs` are
`elixir_file_detection_and_capabilities`,
`elixir_greeting_highlights_language_constructs`,
`elixir_additional_literals_attributes_and_unicode`,
`elixir_incremental_reparse_matches_fresh_through_incomplete_code`, and
`elixir_markdown_fence_uses_bounded_injection`. The grammar-load matrix in
`tests/syntax.rs` and the line-comment inventory in `src/syntax/mod.rs` also
cover registration.

Known limitation: Dedicated text-object, outline, indentation, and fold
queries are absent; ordinary syntax-tree selection remains available. Elixir
language-server integration and EEx/HEEx template grammars are separate work.
Markdown injection retains the existing 128 KB document limit.

## Report

At report time, Runyte did not bundle an Elixir Tree-sitter grammar or register
Elixir in `src/syntax/grammars.rs`. Elixir source files therefore lacked
language-specific syntax highlighting.

Expected behavior was a bundled language named `elixir`, with automatic
detection for `.ex` and `.exs` files, including `mix.exs`, `config.exs`, and
`.formatter.exs`. Markdown code fences labelled `elixir` were expected to use the same
grammar within the existing injection limits.

The reproduction opened `example.ex` with the following content, then opened
the same content as `example.exs`:

```elixir
defmodule Example do
  # Return greetings for the supplied names.
  def greet(names) when is_list(names) do
    names
    |> Enum.map(fn name -> {:ok, "Hello, #{name}!"} end)
  end
end
```

The files had no Elixir-specific highlighting. Expected highlighting distinguished
comments, keywords, module aliases, function definitions and calls, atoms,
strings and interpolation, numbers, and operators. Requested coverage also
included module attributes, pattern matching, sigils, and multiline strings,
with highlighting remaining correct after incremental edits and while code was
temporarily incomplete.

The grammar was required to follow the existing statically linked Tree-sitter
integration and map captures to Runyte's existing scopes. The report required
verification of grammar and query versions, compatibility with the pinned
parsing stack, and licensing. Highlighting was expected to work offline without
an Elixir runtime or language server. Regression coverage called for file
detection, query compilation, representative highlight spans, incremental
edits, and Markdown fence injection, plus updates to the documented language
list and bundled grammar count.

Dedicated text-object, outline, indentation, and fold queries were left as an
implementation scope decision; unsupported capabilities needed documentation.
Elixir language-server integration and EEx/HEEx template grammars are separate
work from this Tree-sitter language addition.

Language reference: [official Elixir documentation](https://elixir-lang.org/docs/).
