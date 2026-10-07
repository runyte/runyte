---
title: "JSONC files are not recognized and cannot be given a language server"
status: resolved
reported: 2026-10-07
resolved: 2026-10-07
commit: d93a46e
---

## Resolution

Commit `d93a46e` (Add JSONC language support) registered a `jsonc` language
in `BUILTIN_LANGUAGES` in `src/syntax/grammars.rs`. Previously the only JSON
definition was `json`, detected by the `.json` extension alone, so
`Registry::language_for_path` returned no language for a `.jsonc` file. The
buffer opened as plain text, no language server attached, and the
configuration reader in `src/config.rs` rejected `lsp.jsonc` because it accepts
only built-in language names.

The bundled `tree-sitter-json 0.24.8` grammar already parses `//` and `/* */`
comments as extras, and its packaged highlight query captures them, so `jsonc`
reuses the JSON grammar, highlights, indentation, and fold queries rather than
adding a parser. The shared fragments are named constants so both definitions
compose the same sources. A separate language rather than an extra extension
on `json` was chosen for two reasons: language servers receive the Runyte
language name as `languageId`, and servers such as Biome and
vscode-json-languageserver accept comments only under `jsonc`; and the line
comment marker differs, `//` for JSONC against none for JSON.

Detection covers the `.jsonc` extension and the exact file names
`tsconfig.json`, `jsconfig.json`, `devcontainer.json`, and
`.devcontainer.json`, whose tools document comment support. Those four names
were previously JSON; a server configured only under `lsp.json` no longer
attaches to them.

The tests are `jsonc_file_detection_and_comment_marker`,
`jsonc_highlights_line_and_block_comments_without_parse_errors`, and
`jsonc_markdown_fence_uses_the_jsonc_language` in `tests/syntax.rs`, and
`jsonc_takes_its_own_language_server_beside_json` in `src/config.rs`. The
detection inventory in `src/syntax/grammars.rs`, the line-comment inventory in
`src/syntax/mod.rs`, and the grammar-load, path-detection, and indentation and
fold matrices in `tests/syntax.rs` also cover registration.

Known limitation: Detection sees only the file name, so JSONC files that keep
a generic `.json` name, such as `.vscode/settings.json` or
`tsconfig.base.json`, stay JSON; there is no user-configurable file-type
mapping. The grammar does not accept trailing commas: a file with one still
highlights, but its tree carries a parse error and some folds are missing.
Runyte has no formatter setting other than a language server, and `:pipe`
does not expand the buffer path.

## Report

At report time, Runyte had no way to configure `.jsonc` support. A `.jsonc`
file was not recognized as any language, and the configuration offered no
mapping from an extension to a language. The report also asked whether an
external formatter such as `biome` could be used.

Expected behavior was JSONC highlighting for `.jsonc` files and a way to format
them with Biome. Biome provides a language server started as
`biome lsp-proxy`, so formatting is available through `:format` once a server
can be configured for the language. Until a JSONC language existed, the
available workaround was filtering the whole buffer through Biome's stdin
mode:

```yaml
keys:
  bind:
    normal:
      F9: [select-all, { command: pipe, argument: "biome format --stdin-file-path=file.jsonc" }]
```

`:pipe` does not expand editor variables, so the path given to
`--stdin-file-path` is fixed; Biome uses it only to choose the parser and match
configuration overrides, which limits one binding to one file type. A
dedicated formatter command per language and format on save were not decided
by the report.
