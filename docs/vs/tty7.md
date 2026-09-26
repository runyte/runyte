# Runyte vs tty7

[All comparisons](README.md) · Last verified: 2026-09-26

Scope: Runyte in this repository and tty7’s published documentation checked on the date above; no comparative performance measurements.

## Summary

tty7 is a graphical terminal workbench with its own GPU-rendered window. Runyte runs inside an existing terminal and makes a modal editor the centre of its workspace. Both can host coding agents and retain terminal processes independently of the visible interface. [tty7 overview](https://github.com/l0ng-ai/tty7).

This comparison spans different layers: tty7 can provide the terminal in which Runyte runs. Its command-input editing is distinct from Runyte’s project-file editor.

## Feature comparison

Runyte capabilities follow the [project overview](../../README.md),
[user guide](../user-guide.md), [plugin contract](../plugins.md), and
[optional context bridge](../../bridges/runyte-context/README.md).
The bridge requires native grants; terminal-text proposals need approval and do not submit Enter.

| Area | Runyte | tty7 |
| --- | --- | --- |
| Primary purpose | Modal editor and integrated project workspace. | Graphical terminal workbench for shells, remote work, and agents. |
| Editing | Selection-first editing, multiple selections, registers, and macros. | Enhanced shell input; use a separate editor for project files. |
| Terminals | Built-in terminal panes with scrollback and modal review. | Own terminal window, tabs, splits, and background server. |
| Navigation and search | Finder searches files, unsaved buffers, and terminals together. | Scrollback search, command palette, and repository-grouped tabs. |
| Persistence | Optional persistent host retains buffers and processes while detached; live state ends at host shutdown or reboot. | Background server retains shells when the window closes; supported agent sessions can resume after restart. Resumption uses new processes. |
| Agent integration | Run agents in terminals; optional MCP bridge grants access to buffers and terminal context. | Agent detection; supported hooks add status, notifications, waits, and resumption. |
| Extensibility | Explicitly enabled external-process plugins, in any language, using the documented JSON protocol. | CLI automation and agent hooks; these differ from an editor plugin API. |
| Interaction model | Modal keys, contextual help and key hints, plus mouse selection and pane resizing. | Graphical controls and keyboard shortcuts; GPU rendering via gpui. |
| Remote work | Workspace host uses local transport. | Native SSH stack, remote workspaces, SFTP, and port forwarding. |

The tty7 column follows its [official README](https://github.com/l0ng-ai/tty7). Detection and hook-supported status are separate capabilities; check its per-agent matrix. Reboot recovery must not be read as original shell processes surviving a reboot.

## Use cases

These are workflow recommendations based on the capabilities above.

- **Runyte:** You want modal project editing within the terminal environment you already use.
- **tty7:** You want a graphical terminal application that manages remote connections and agent activity.
- **Together:** Runyte could be your editor inside tty7. This is a composition option, not a validated integration; let each layer own a clear part of the layout.
