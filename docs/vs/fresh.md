# Runyte vs Fresh

[All comparisons](README.md) · Last verified: 2026-09-26

Scope: Runyte in this repository and Fresh’s published documentation checked on the date above; no comparative performance measurements.

## Summary

Fresh and Runyte overlap closely: both combine terminal-based editing, language tooling, file navigation, integrated terminals, and persistent editor processes. Fresh emphasizes familiar editor controls and menus; Runyte emphasizes selection-first modal editing. [Fresh documentation](https://getfresh.dev/docs/).

Persistence alone does not distinguish them. Fresh also documents daemon attachment and restoration of unsaved buffers after quitting, while Runyte’s persistent mode retains live workspace state until its host stops.

## Feature comparison

Runyte capabilities follow the [project overview](../../README.md),
[user guide](../user-guide.md), [plugin contract](../plugins.md), and
[optional context bridge](../../bridges/runyte-context/README.md).
The bridge requires native grants; terminal-text proposals need approval and do not submit Enter.

| Area | Runyte | Fresh |
| --- | --- | --- |
| Primary purpose | Modal editor and integrated project workspace. | Terminal editor and IDE with familiar editor controls. |
| Editing | Selection-first editing, multiple selections, registers, and macros. | Multiple cursors and conventional editing operations. [Features](https://getfresh.dev/docs/features/). |
| Terminals | Built-in terminal panes with scrollback and modal review. | Built-in terminal emulator. [Terminal](https://getfresh.dev/docs/features/terminal). |
| Navigation and search | Finder searches files, unsaved buffers, and terminals together. | File explorer, navigation, and project search; bundled plugins add Git search tools. [Plugins](https://getfresh.dev/docs/plugins/). |
| Persistence | Optional persistent host retains buffers and processes while detached; live state ends at host shutdown or reboot. | Daemon mode supports detach/reattach; hot exit restores unsaved buffers. A bare invocation defaults to its orchestrator workspace. [Daemon mode](https://getfresh.dev/docs/features/session-persistence). |
| Agent integration | Run agents in terminals; optional MCP bridge grants access to buffers and terminal context. | Terminal-hosted agents and CLI scripting that can inspect and manipulate the editor through TypeScript. [Scripting](https://getfresh.dev/docs/features/scripting). |
| Extensibility | Explicitly enabled external-process plugins, in any language, using the documented JSON protocol. | TypeScript plugins, language packs, themes, and a built-in package manager. [Plugins](https://getfresh.dev/docs/plugins/). |
| Interaction model | Modal keys, contextual help and key hints, plus mouse selection and pane resizing. | Conventional shortcuts, command palette, menus, mouse interaction, and a keybinding editor. [Features](https://getfresh.dev/docs/features/). |
| Language tooling | Bundled Tree-sitter grammars and asynchronous LSP. | Syntax highlighting and LSP integration. [Documentation](https://getfresh.dev/docs/). |
| Remote work | Persistent-session transport is local; remote document providers can be implemented by plugins. | Documents SSH editing and devcontainer backends. [Features](https://getfresh.dev/docs/features/). |

## Use cases

These are workflow recommendations based on the capabilities above.

- **Runyte:** You prefer a selection-first modal workflow and unified search across project files, unsaved buffers, and terminal text.
- **Fresh:** You want a terminal editor with familiar controls, TypeScript customization, and documented hot-exit recovery.
- **Evaluation:** Try the same editing and terminal task in both. The meaningful choice is interaction and workspace behavior, not simply whether either has terminals or persistence.
