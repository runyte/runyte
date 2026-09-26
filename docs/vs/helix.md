# Runyte vs Helix

[All comparisons](README.md) · Last verified: 2026-09-26

Scope: Runyte in this repository and Helix’s published documentation checked on the date above; no comparative performance measurements.

## Summary

Helix and Runyte share a selection-first approach, multiple selections, and integrated language tooling. Helix concentrates on editing; Runyte also owns terminal panes, Git workflows, and persistent workspaces. [Helix overview](https://helix-editor.com/).

Runyte draws from Helix but is not keymap-compatible. Search and macros deliberately differ; consult the [keymap register](../../context/reference/helix-keymap-v1.md) before transferring muscle memory.

## Feature comparison

Runyte capabilities follow the [project overview](../../README.md),
[user guide](../user-guide.md), [plugin contract](../plugins.md), and
[optional context bridge](../../bridges/runyte-context/README.md).
The bridge requires native grants; terminal-text proposals need approval and do not submit Enter.

| Area | Runyte | Helix |
| --- | --- | --- |
| Primary purpose | Modal editor and integrated project workspace. | Modal text editor with integrated language features. |
| Editing | Selection-first editing, multiple selections, registers, and macros. | Selection-first commands and multiple selections as core primitives. |
| Terminals | Built-in terminal panes with scrollback and modal review. | Use a surrounding terminal or multiplexer; no integrated terminal emulator is documented. |
| Navigation and search | Finder searches files, unsaved buffers, and terminals together. | File and symbol pickers and project search. |
| Persistence | Optional persistent host retains buffers and processes while detached; live state ends at host shutdown or reboot. | Use an external multiplexer to keep the editor running when detaching; no comparable built-in workspace host is documented. |
| Agent integration | Run agents in terminals; optional MCP bridge grants access to buffers and terminal context. | Use agents alongside the editor through external tools; no equivalent built-in context bridge is documented. |
| Extensibility | Explicitly enabled external-process plugins, in any language, using the documented JSON protocol. | Configuration and shell commands; the official FAQ says a plugin system is not yet available. [FAQ](https://helix-editor.com/). |
| Interaction model | Modal keys, contextual help and key hints, plus mouse selection and pane resizing. | Modal keys, command completion, and discoverable command menus. [Keymap](https://docs.helix-editor.com/keymap.html). |
| Language tooling | Bundled Tree-sitter grammars and asynchronous LSP; language servers need workspace permission. | Built-in Tree-sitter and LSP integration; language-server executables are installed separately. [Overview](https://helix-editor.com/). |

Helix cells describe the [official feature overview and FAQ](https://helix-editor.com/), not proposed plugin work.

## Use cases

These are workflow recommendations based on the capabilities above.

- **Runyte:** You want selection-first editing and a shared workflow for files, Git, terminals, and agents.
- **Helix:** You want a focused selection-first editor and already have a preferred shell or multiplexer setup.
- **Migration:** Treat Runyte as a distinct editor. In particular, its literal and regex searches select all matches rather than reproducing Helix’s search behavior.
