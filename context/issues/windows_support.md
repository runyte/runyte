Windows support is incomplete and is not currently a first-class project goal.
Runyte may omit or disable features on Windows when a sound implementation would
be disproportionately difficult. Access to Windows for manual testing is
limited, so platform-specific behavior must fail clearly and leave user data
intact when it cannot be exercised regularly.

The first task is to scan the entire Runyte source tree, dependencies, build and
packaging configuration for Windows compatibility problems. The checklist below
is only an initial set of likely areas; it must not be treated as complete until
that scan has been performed and its findings have been added here.

- Add a Windows equivalent of the Linux `xdg-open` and macOS `open` behavior for
  opening binary files with the user's system-default application. Research a
  direct, argument-safe Windows API or executable invocation rather than using a
  shell, and preserve the explicit application prompt as a fallback.
- Audit the persistent workspace host, local protocol, endpoint discovery, and
  `--wait` lifecycle for Unix socket and process-model assumptions. It is
  acceptable to disable persistent hosting or fall back to an in-process mode
  on Windows if the full lifecycle is not practical.
- The workspace commands added by `workspace_switching.md` stay in the command
  inventory on every platform and report themselves unavailable off Unix,
  rather than being compiled out. `COMMANDS` feeds the command palette, help,
  and key dispatch from one table, and several tests pin its exact size, so a
  platform-conditional inventory would make those counts and
  `EditorCommand::ALL` differ per platform and would introduce the first
  conditional compilation into `src/command.rs` and `src/keymap.rs`. Reporting
  unavailability reuses `CommandAvailability::Unavailable`, which already
  explains a dimmed `:format` when no language server is configured, and it
  tells a Windows user why a command cannot run instead of hiding it. Apply the
  same approach to any later feature that exists only on one platform.
- Audit process spawning, signals, terminal/PTY handling, shell integration,
  executable discovery, and detached child behavior.
- Audit filesystem paths, non-UTF-8 assumptions, separators and prefixes,
  symlinks, permissions, atomic replacement, locking, trash behavior, and
  temporary-file handling.
- Audit Git, language-server, and clipboard helpers for Unix
  commands or conventions. Features without a dependable Windows equivalent
  may be reported as unavailable.
- Add Windows-aware installation and release packaging, and at least a
  cross-compilation or CI build check where practical.
- Define a small manual smoke-test list that fits the limited Windows testing
  available, with automated tests for platform selection and graceful
  unsupported-feature errors wherever possible.
