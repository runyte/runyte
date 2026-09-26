# Runyte vs Zellij

[All comparisons](README.md) · Last verified: 2026-09-26

Scope: Runyte in this repository and Zellij’s published documentation checked on the date above; no comparative performance measurements.

## Summary

Zellij and Runyte both offer terminal workspaces with panes, discoverable controls, and extensions. Zellij concentrates on arranging terminal applications and plugin panes; Runyte builds the workspace around its native editor. [Zellij guide](https://zellij.dev/documentation/).

They overlap in persistence and navigation, but a multiplexer does not own the editable buffers inside its child editors. Runyte can search its own unsaved text together with files and terminal output.

## Feature comparison

Runyte capabilities follow the [project overview](../../README.md),
[user guide](../user-guide.md), [plugin contract](../plugins.md), and
[optional context bridge](../../bridges/runyte-context/README.md).
The bridge requires native grants; terminal-text proposals need approval and do not submit Enter.

| Area | Runyte | Zellij |
| --- | --- | --- |
| Primary purpose | Modal editor and integrated project workspace. | Terminal workspace for applications and plugins. |
| Editing | Selection-first editing, multiple selections, registers, and macros. | Use a separate editor in a terminal pane. |
| Terminals | Built-in terminal panes with scrollback and modal review. | Tabs with tiled, floating, and stacked panes; reusable layouts. [Features](https://zellij.dev/features/). |
| Navigation and search | Finder searches files, unsaved buffers, and terminals together. | Pane/tab navigation and bundled Strider file picker; open scrollback in your chosen editor. [Features](https://zellij.dev/features/). |
| Persistence | Optional persistent host retains buffers and processes while detached; live state ends at host shutdown or reboot. | Detach/reattach plus session resurrection: restore layouts and optionally terminal history, then rerun commands. [Resurrection](https://zellij.dev/documentation/session-resurrection). |
| Agent integration | Run agents in terminals; optional MCP bridge grants access to buffers and terminal context. | Run agents in panes; CLI actions and plugins provide automation. [Features](https://zellij.dev/features/). |
| Extensibility | Explicitly enabled external-process plugins, in any language, using the documented JSON protocol. | WebAssembly/WASI plugins that can render panes and control the workspace. [Plugins](https://zellij.dev/documentation/plugins). |
| Interaction model | Modal keys, contextual help and key hints, plus mouse selection and pane resizing. | Keyboard modes and visible key guidance, with mouse interaction. |
| Remote work | Persistent-session transport is local to the workspace host. | Built-in web client and remote attachment over HTTPS. [Features](https://zellij.dev/features/). |

See the [Zellij user guide](https://zellij.dev/documentation/) for workspace controls. Session resurrection recreates a workspace; it does not preserve the original processes across a reboot.

## Use cases

These are workflow recommendations based on the capabilities above.

- **Runyte:** You want native editing and integrated project navigation alongside persistent terminals.
- **Zellij:** You want a terminal workspace with configurable layouts and plugin panes while keeping your chosen editor.
- **Together:** Running Runyte inside Zellij is a composition option. Decide which layer owns splits and shortcuts; resurrecting a terminal command does not restore that editor’s previous in-memory buffers.
