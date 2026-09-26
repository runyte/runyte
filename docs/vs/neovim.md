# Runyte vs Neovim

[All comparisons](README.md) · Last verified: 2026-09-26

Scope: Runyte in this repository and Neovim’s published documentation checked on the date above; no comparative performance measurements. Neovim’s live web manual tracks development; check installed-version help before relying on newer features.

## Summary

Neovim and Runyte both combine modal editing, language tooling, terminal emulation, and programmable extensions. Neovim builds on Vim’s editing language and offers extensive customization through Lua, Vimscript, and APIs. [Neovim overview](https://neovim.io/).

Runyte supplies a selection-first project workflow with its own Finder, Git views, and session manager. The useful distinction is how you want to assemble and control that workspace, rather than whether an editor can host a terminal.

## Feature comparison

Runyte capabilities follow the [project overview](../../README.md),
[user guide](../user-guide.md), [plugin contract](../plugins.md), and
[optional context bridge](../../bridges/runyte-context/README.md).
The bridge requires native grants; terminal-text proposals need approval and do not submit Enter.

| Area | Runyte | Neovim |
| --- | --- | --- |
| Primary purpose | Modal editor and integrated project workspace. | Extensible Vim-based editor, usable through terminal or external graphical UIs. [Overview](https://neovim.io/). |
| Editing | Selection-first editing, multiple selections, registers, and macros. | Vim operators, motions, Visual mode, registers, and macros; current documentation also advertises multicursor support. [Overview](https://neovim.io/). |
| Terminals | Built-in terminal panes with scrollback and modal review. | Built-in terminal buffers, displayed in editor windows. [Terminal](https://neovim.io/doc/user/terminal/). |
| Navigation and search | Finder searches files, unsaved buffers, and terminals together. | Buffer/window navigation and search; plugins can provide additional project pickers and combined views. |
| Persistence | Optional persistent host retains buffers and processes while detached; live state ends at host shutdown or reboot. | Client/server and remote UI attachment are supported. Saved terminal buffers restart commands when restored; that differs from keeping processes alive. [Remote](https://neovim.io/doc/user/remote/), [terminal](https://neovim.io/doc/user/terminal/). |
| Agent integration | Run agents in terminals; optional MCP bridge grants access to buffers and terminal context. | Agents can run in terminal buffers; editor automation uses APIs, RPC, and optional integrations. |
| Extensibility | Explicitly enabled external-process plugins, in any language, using the documented JSON protocol. | Lua, Vimscript, and RPC extensions in other languages. [Overview](https://neovim.io/). |
| Interaction model | Modal keys, contextual help and key hints, plus mouse selection and pane resizing. | Vim-style modal interaction, configurable mappings, and interfaces supplied by the chosen UI. |
| Language tooling | Bundled Tree-sitter grammars and an asynchronous LSP client. | Built-in LSP and Tree-sitter facilities; configuration and plugins shape the language workflow. [Overview](https://neovim.io/). |

## Use cases

These are workflow recommendations based on the capabilities above.

- **Runyte:** You want an integrated selection-first workspace with consistent discovery across files, terminals, and Git.
- **Neovim:** You want to retain Vim habits or build a tailored environment around Lua, Vimscript, RPC, and existing plugins.
- **Evaluation:** Compare Runyte with the Neovim configuration you would actually use. Plugin-provided capabilities should be credited without describing them as defaults.
