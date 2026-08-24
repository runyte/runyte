---
title: "Markdown structure and inline elements were not usefully colored"
status: resolved
reported: 2026-08-19
resolved: 2026-08-19
legacy_commit: 9f18871
---

## Resolution

Commit `9f18871` (`Color Markdown structure and inline syntax`) fixed two gaps
in the syntax boundary. `Registry::new_with_overrides` registered Markdown's
block grammar but not the separate `markdown_inline` grammar requested by its
injection query, so emphasis, strong text, links, and inline code never reached
a parser that could identify them. `scope_for_capture` also had no semantic
scope corresponding to the upstream block query's `text.*` captures, so
heading text and other recognized block content fell back to the ordinary
foreground.

The syntax registry now admits an internal injection-only language and maps it
back to the public Markdown identity without exposing it to file detection,
language lookup, or LSP configuration. Runyte-owned Markdown injection and
highlight queries retain inline children for the second parser and emit eight
themeable `markup.*` roles for headings, bold and italic text, links, lists,
quotes, and raw text. Bundled themes derive those roles from their existing
semantic palettes; old custom themes remain valid and may override the new
roles individually.

Runyte's presentation model currently carries foreground colors rather than
font modifiers. Consequently `**text**` and `__text__` use the `markup.bold`
color, while `*text*` and `_text_` use the `markup.italic` color; they are not
rendered with terminal bold and italic attributes. Backtick code uses the
separate `markup.raw` color.

Coverage lives in:

- `tests/syntax.rs`: `markdown_highlights_structure`
- `tests/syntax.rs`: `large_markdown_keeps_block_color_and_drops_inline_color_with_injections`
- `src/syntax/mod.rs`: `markdown_inline_is_an_internal_layer_of_the_public_markdown_language`
- `src/syntax/mod.rs`: `every_canonical_plain_and_owned_capability_query_compiles`
- `src/config.rs`: `bundled_themes_color_every_semantic_markdown_scope`

Known limitation: documents above 128 KB continue to use Runyte's
injection-free syntax tree. Their headings, lists, quotes, and other block
structure stay colored, but emphasis, strong text, links, and inline code use
the ordinary foreground.

## Report

Markdown files lacked useful coloring. Long technical documents were difficult
to read without colored section titles and other Markdown elements.

During the implementation discussion, support was explicitly requested for
`*text*`, `**text**`, and `` `text` ``.
