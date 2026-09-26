# Runyte vs tmux

[All comparisons](README.md) · Last verified: 2026-09-26

Scope: Runyte in this repository and tmux’s published documentation checked on the date above; no comparative performance measurements.

## Summary

tmux organizes terminal programs and keeps them running when you detach. Runyte provides that kind of continuity in its optional persistent mode while also owning editable documents and project tools. [tmux introduction](https://github.com/tmux/tmux/wiki).

Both can host shells, tests, and coding agents. tmux lets you select every application inside the workspace; Runyte supplies an editor and understands its buffers as well as terminal output.

## Feature comparison

Runyte capabilities follow the [project overview](../../README.md),
[user guide](../user-guide.md), [plugin contract](../plugins.md), and
[optional context bridge](../../bridges/runyte-context/README.md).
The bridge requires native grants; terminal-text proposals need approval and do not submit Enter.

| Area | Runyte | tmux |
| --- | --- | --- |
| Primary purpose | Modal editor and integrated project workspace. | General-purpose terminal multiplexer. |
| Editing | Selection-first editing, multiple selections, registers, and macros. | Run a separate editor in a pane; copy mode operates on terminal history. |
| Terminals | Built-in terminal panes with scrollback and modal review. | Sessions, windows, and panes for arbitrary terminal programs. |
| Navigation and search | Finder searches files, unsaved buffers, and terminals together. | Navigate sessions/windows/panes and search terminal history; project-file search belongs to hosted tools. |
| Persistence | Optional persistent host retains buffers and processes while detached; live state ends at host shutdown or reboot. | Detach and reattach to the live server. Core live processes do not survive server termination or reboot. |
| Agent integration | Run agents in terminals; optional MCP bridge grants access to buffers and terminal context. | Run agents as terminal programs; scripts can capture output and send input. Agent-specific behavior comes from additional tooling. |
| Extensibility | Explicitly enabled external-process plugins, in any language, using the documented JSON protocol. | Commands, hooks, shell scripts, and external plugin tooling. |
| Interaction model | Modal keys, contextual help and key hints, plus mouse selection and pane resizing. | Prefix bindings, command prompt, copy mode, and optional mouse control. |

The tmux column follows the official [getting-started guide](https://github.com/tmux/tmux/wiki/Getting-Started). Its program-management features are useful independently of any editor.

## Use cases

These are workflow recommendations based on the capabilities above.

- **Runyte:** You want the editor to own navigation across code, scratch buffers, Git views, and terminal output.
- **tmux:** You need persistent shells and arbitrary terminal applications, while choosing and configuring your editor independently.
- **Together:** Runyte can sit inside tmux. Standalone Runyte may be sufficient when tmux owns detach/reattach; check outer keybindings because they can intercept editor shortcuts.
