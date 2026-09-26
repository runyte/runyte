# Runyte vs Herdr

[All comparisons](README.md) · Last verified: 2026-09-26

Scope: Runyte in this repository and Herdr’s published documentation checked on the date above; no comparative performance measurements.

## Summary

Herdr focuses on supervising coding agents. Runyte focuses on editing and navigating the project those agents are working on. Both combine real terminal processes with detachable workspaces. [Herdr overview](https://github.com/herdrdev/herdr).

The most visible distinction is agent status: Herdr helps identify which agent needs attention, while Runyte puts editable documents, Git views, and terminal text in one navigation workflow.

## Feature comparison

Runyte capabilities follow the [project overview](../../README.md),
[user guide](../user-guide.md), [plugin contract](../plugins.md), and
[optional context bridge](../../bridges/runyte-context/README.md).
The bridge requires native grants; terminal-text proposals need approval and do not submit Enter.

| Area | Runyte | Herdr |
| --- | --- | --- |
| Primary purpose | Modal editor and integrated project workspace. | Agent runtime and terminal workspace. [Overview](https://github.com/herdrdev/herdr). |
| Editing | Selection-first editing, multiple selections, registers, and macros. | Run your chosen editor in a terminal pane. |
| Terminals | Built-in terminal panes with scrollback and modal review. | Real terminal panes grouped into tabs and workspaces. |
| Navigation and search | Finder searches files, unsaved buffers, and terminals together. | Workspace, tab, and agent navigation, with status rolled up across projects. [Agents](https://herdr.dev/docs/agents/). |
| Persistence | Optional persistent host retains buffers and processes while detached; live state ends at host shutdown or reboot. | Background server by default; saved layouts and supported agent sessions can be restored after restart, using new processes. [Overview](https://github.com/herdrdev/herdr). |
| Agent integration | Run agents in terminals; optional MCP bridge grants access to buffers and terminal context. | Agent detection, state notifications, and CLI/socket automation. Screen-based detection can miss unfamiliar prompts. [Agents](https://herdr.dev/docs/agents/). |
| Extensibility | Explicitly enabled external-process plugins, in any language, using the documented JSON protocol. | Plugins plus CLI and socket API integrations. [Overview](https://github.com/herdrdev/herdr). |
| Interaction model | Modal keys, contextual help and key hints, plus mouse selection and pane resizing. | Mouse interaction and tmux-style prefix bindings. [Quick start](https://herdr.dev/docs/quick-start/). |
| Remote work | Persistent-session transport is local to the workspace host. | Combines local and saved SSH machines in one interface. [Overview](https://github.com/herdrdev/herdr). |

## Use cases

These are workflow recommendations based on the capabilities above.

- **Runyte:** You alternate between writing code, reviewing changes, composing prompts, and searching agent output.
- **Herdr:** You supervise several agents or machines and want to see which needs a decision without visiting every pane.
- **Together:** Runyte could serve as the editor inside a Herdr pane. That is a composition option, not a tested integration; choose which layer owns terminal layout and persistence.
