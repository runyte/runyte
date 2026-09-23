Runyte does not bundle an Elixir Tree-sitter grammar or register Elixir in
`src/syntax/grammars.rs`. Elixir source files therefore lack language-specific
syntax highlighting.

Elixir should be available as a bundled language named `elixir`, with automatic
detection for `.ex` and `.exs` files, including `mix.exs`, `config.exs`, and
`.formatter.exs`. Markdown code fences labelled `elixir` should use the same
grammar within the existing injection limits.

To reproduce, open `example.ex` with the following content, then open the same
content as `example.exs`:

```elixir
defmodule Example do
  # Return greetings for the supplied names.
  def greet(names) when is_list(names) do
    names
    |> Enum.map(fn name -> {:ok, "Hello, #{name}!"} end)
  end
end
```

The files currently have no Elixir-specific highlighting. Expected highlighting
should distinguish comments, keywords, module aliases, function definitions and
calls, atoms, strings and interpolation, numbers, and operators. Coverage should
also include module attributes, pattern matching, sigils, and multiline strings,
with highlighting remaining correct after incremental edits and while code is
temporarily incomplete.

The grammar must follow the existing statically linked Tree-sitter integration
and map captures to Runyte's existing scopes. Grammar and query versions,
compatibility with the pinned parsing stack, and licensing must be verified
during implementation. Highlighting must work offline without an Elixir runtime
or language server. Regression coverage should exercise file detection, query
compilation, representative highlight spans, incremental edits, and Markdown
fence injection. Update the documented language list and bundled grammar count.

Dedicated text-object, outline, indentation, and fold queries remain a scope
decision for implementation; any unsupported capabilities should be documented.
Elixir language-server integration and EEx/HEEx template grammars are separate
work from this Tree-sitter language addition.

Language reference: [official Elixir documentation](https://elixir-lang.org/docs/).
