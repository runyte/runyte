# Runyte user guide

This is the detailed reference for Runyte. For the project overview and quick
start, see the [main README](../../README.md).

The guide is best read with an AI agent: ask it specific questions rather than
reading from top to bottom.

Start with [Install and run](install-and-run.md), learn [editing and search](key-bindings.md),
or find [files](files-and-buffers.md), [terminal sessions](terminals.md), and [settings](configuration.md).

## Contents

- [Overview — What Runyte does, and how it relates to Helix](overview.md)
- [Install and run — Installing, updating, Windows, and launching](install-and-run.md)
- [The screen — Layout, status line, notifications, help, mouse](the-screen.md)
- [Key bindings — Editing, Insert mode, search, layout, text objects, macros](key-bindings.md)
- [Files and buffers — Finder, explorer, directory tree, comparisons, pipes](files-and-buffers.md)
- [Panes — Splits, focus, resizing, zen and fullscreen](panes.md)
- [Terminals — Integrated terminal sessions](terminals.md)
- [Git — Status, branches, worktrees, history, staging, pull and push](git.md)
- [Language support — Syntax highlighting, rendered Markdown, language servers](language-support.md)
- [Workspaces and persistent sessions — Editor, ide, and mux modes, the session manager](workspaces-and-persistent-sessions.md)
- [Commands — The command palette and every `:` command](commands.md)
- [Plugins — Process plugins and application views](plugins.md)
- [Agent access to workspace context — The built-in MCP adapter for AI agents](agent-access-to-workspace-context.md)
- [Diagnostics and logging — The diagnostic log](diagnostics-and-logging.md)
- [Configuration — The configuration file and every setting](configuration.md)
- [Custom key bindings — Remapping keys and binding actions](custom-key-bindings.md)
- [Themes — Built-in and custom themes](themes.md)
- [Project layout](project-layout.md)
- [License](license.md)

## Maintaining the guide

These chapters are the authoritative guide for the repository, embedded `:help`,
and runyte.com. The Contents list above defines their assembly order. Keep chapter
headings and anchors stable; links may point directly to another chapter.
Run `python3 contrib/check_user_guide.py` from the repository root to check
chapter navigation and local links.
The website importer reads this checkout; do not hand-edit imported website data.

Related references: [installation](../../INSTALLATION.md), [plugins](../plugins.md),
[MCP](../mcp.md), and [FAQ](../faq.md).
