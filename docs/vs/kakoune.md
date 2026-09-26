# Runyte vs Kakoune

[All comparisons](README.md) · Last verified: 2026-09-26

Scope: Runyte in this repository and Kakoune’s published documentation checked on the date above; no comparative performance measurements.

## Summary

Kakoune and Runyte both make selections central to editing. Kakoune emphasizes composing an editor with Unix tools; Runyte integrates more of the surrounding workspace. [Kakoune design](https://kakoune.org/why-kakoune/why-kakoune.html).

Both expose visible selections before an operation and support external automation. Their commands and integration boundaries differ, so a shared editing philosophy does not imply interchangeable bindings.

## Feature comparison

Runyte capabilities follow the [project overview](../../README.md),
[user guide](../user-guide.md), [plugin contract](../plugins.md), and
[optional context bridge](../../bridges/runyte-context/README.md).
The bridge requires native grants; terminal-text proposals need approval and do not submit Enter.

| Area | Runyte | Kakoune |
| --- | --- | --- |
| Primary purpose | Modal editor and integrated project workspace. | Modal editor designed to cooperate with external tools. |
| Editing | Selection-first editing, multiple selections, registers, and macros. | Selection-then-action editing with multiple selections at its centre. |
| Terminals | Built-in terminal panes with scrollback and modal review. | Use an external terminal or multiplexer for interactive programs. |
| Navigation and search | Finder searches files, unsaved buffers, and terminals together. | Buffer navigation, regex selection, completion, and external search integrations. |
| Persistence | Optional persistent host retains buffers and processes while detached; live state ends at host shutdown or reboot. | Client/server editing sessions can serve multiple clients; window arrangement is delegated to the surrounding environment. [Design](https://kakoune.org/why-kakoune/why-kakoune.html). |
| Agent integration | Run agents in terminals; optional MCP bridge grants access to buffers and terminal context. | Shell-driven editor commands and external integrations; agents can run beside the editor. |
| Extensibility | Explicitly enabled external-process plugins, in any language, using the documented JSON protocol. | Kakoune commands, hooks, shell expansion, and asynchronous external tools. |
| Interaction model | Modal keys, contextual help and key hints, plus mouse selection and pane resizing. | Modal keys with completion and contextual command information. |
| Language tooling | Built-in LSP client and Tree-sitter integration. | LSP is provided by the separate kakoune-lsp project. [kakoune-lsp](https://github.com/kakoune-lsp/kakoune-lsp). |

Kakoune’s [design explanation](https://kakoune.org/why-kakoune/why-kakoune.html) describes the composition model behind these rows; the [project repository](https://github.com/mawww/kakoune) is the current implementation reference.

## Use cases

These are workflow recommendations based on the capabilities above.

- **Runyte:** You like multiple selections and want terminal layout, project search, and Git workflows managed by the editor.
- **Kakoune:** You want a selection-oriented editor that you compose with shell scripts, a window manager, and language-tool integrations.
- **Together:** Kakoune can be used as a separate editor in a terminal pane, but choose one editor to own a given unsaved document to avoid competing edits.
