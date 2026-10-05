# Launch modes are hard to tell apart and change by launch directory

Runyte currently has three ways a session can run, described by two unrelated
axes:

- **standalone** and **persistent** (`workspace.mode`, `--standalone`,
  `--persistent`/`-a`) say where editor state lives: in the TUI process, or in
  a persistent session host that survives the TUI.
- **plain** says there is no workspace at all. A launch naming a file or
  directory becomes plain when no Git root or `.runyte/` is found from the
  launch directory, when `--plain` is given, or when the editor runs as root.
  `:workspace-init` later turns a plain session into a standalone workspace
  session without restarting it.

The names describe the process model rather than what a person gets, and the
split between plain and standalone is unclear: the same `runyte FILE` command
produces a plain or a workspace session depending on the launch directory, the
effective user, and later commands. Upgrading a running plain session to a
workspace also needs a separate path for starting every workspace service
mid-session, which has been the source of most defects in that feature.

`--init DIRECTORY` creates a workspace and then opens the editor in it, so
initializing a workspace and editing in one are a single step. Outside a
workspace, a bare `runyte` stops at an interactive question:

```
No Git repository or existing project workspace directory was found.
Project directory [/path]:
```

## Expected behavior

Runyte has three modes named after what they provide. The mode is chosen by
command-line flag or configuration, never by the launch directory, and a
running session never changes mode.

| Mode | Flag | Status line | Provides |
|---|---|---|---|
| editor | `--editor` | `NOR │ editor │ Directory: /etc` | Editing files and directories with no workspace |
| ide | `--ide` (default) | `NOR │ ide │ Workspace: …` | A workspace with Git, language servers, MCP, plugins and terminals |
| mux | `--mux`, short `-a` | `NOR │ ide+mux │ Workspace: …` | Everything `ide` provides, kept alive in a persistent session host |

`mux` is additive: it is `ide` whose state survives the TUI. The status line
shows only the mode word in the field that currently reads `standalone` or
`persistent`, with no `Mode:` prefix.

### editor

`runyte --editor` is what plain sessions are today, with these differences:

- No terminals. The terminal commands and bindings are unavailable, as Git and
  language-server commands are.
- No way to gain a workspace: `:workspace-init` is removed.
- The status line shows no Git information (branch, change counts) and
  labels the directory `Directory:` rather than `Workspace:`.

It keeps editing, configuration and theme, syntax highlighting, the editable
explorer with confirmed filesystem plans, the directory tree, and the Finder
and project search over the active file's or explorer's directory with the
existing contained-scan guards. It creates no `.runyte/` directory, default
log or recent-workspace record, and starts no Git, language server, plugin,
agent context endpoint or session catalog. Commands that need any of those
refuse with a reason and are greyed out in help and key hints.

### runed

`runed` is a second name for the same binary. Started as `runed` (the file
stem of the program name, so `runed.exe` on Windows), it always runs as
`runyte --editor`, ignoring the configured mode; `--ide` and `--mux` are
rejected under that name. It gives `EDITOR`, `SUDO_EDITOR` and similar
settings a short command that does not depend on configuration:

```sh
export SUDO_EDITOR=runed
sudoedit /etc/fstab
```

`install.sh` and the release archives provide `runed` beside `runyte`: a
symbolic link on Linux and macOS, and a hard link or copy on Windows.
`cargo install` installs only `runyte`; the user guide shows how to add the
link.

### ide and mux outside a workspace

`ide` and `mux` need a workspace: a Git root, or a `.runyte/` directory in
the launch directory or above it. Without one they refuse instead of asking:

```
no workspace here; run runyte --init DIRECTORY to create one
```

The interactive project-directory question is removed. The same refusal
applies to `-a`/`--mux` with no selector when the launch directory has no
workspace, which today makes the launch directory a workspace.

### --init

`runyte --init DIRECTORY` creates `DIRECTORY/.runyte/` (or accepts an existing
one), prints where the workspace was initialized and how to open it, and
exits without starting the editor:

```
initialized a workspace in /path/to/project
open it with: runyte /path/to/project   or   runyte --mux /path/to/project
```

It takes no file targets and no mode flag, and keeps today's protection
against a state directory that overlaps per-user configuration or cache
storage.

### Configuration

A top-level `mode: editor | ide | mux` setting selects the default mode, and
`ide` is the default when it is absent. `workspace.mode` is accepted as a
deprecated alias with its old values mapped (`standalone` to `ide`,
`persistent` to `mux`) and a warning naming the replacement. A command-line
mode flag overrides the setting.

### Running as root

As root, only `editor` runs. `--ide`, `--mux`, and a configured `ide` or
`mux` mode refuse with a message pointing to `runed` and `sudoedit`, because
either would leave root-owned runtime state in a workspace another account
owns. This replaces the current rule that silently makes a root launch plain.
`--wait` already refuses as root.

### Removed

- `--plain`, `--standalone` and `--persistent` (no deprecation period).
- `:workspace-init` and the mid-session workspace upgrade.
- Automatic plain sessions chosen by the launch directory.
- The interactive project-directory question at startup.
- `-a` with no selector creating a workspace in the launch directory.

`--wait` and `--serve` are unchanged and belong to `mux`.

### Naming a directory still creates a workspace

The refusal above covers launches that rely on the launch directory: a bare
`runyte`, `runyte FILE` or `runyte DIRECTORY` in ide mode, and `-a`/`--mux`
with no selector. Requests that name a directory on purpose keep making it a
workspace when it is not one: `runyte -a DIRECTORY` (`--mux DIRECTORY`),
`:session-attach DIRECTORY`, and the explorer's `Tab s` ("open a persistent
session here").

## Constraints

- The plain-session implementation recorded in
  `context/issues/resolved/plain_sessions_without_a_workspace.md` is the
  starting point for editor mode; that record is updated in the same change
  to say what replaced it.
- Key dispatch, help and key hints keep reading from the keymap registry.
  `context/reference/helix-keymap-v1.md` and
  `context/reference/ui-vocabulary.md` are updated for the removed command and
  the status-line words.
- `README.md`, `docs/user-guide.md`, the CLI help, `install.sh` and its
  acceptance tests in `tests/installer/test_install.py`, and the release
  packaging describe or provide `runed`.
- A release note states the removed flags, the `workspace.mode` alias, and the
  `--init` and outside-workspace changes.

## Reproduction

1. In a directory with no Git repository and no `.runyte/`, run
   `runyte notes.txt`: a plain session opens.
2. In a Git repository, run the same command: a workspace session opens.
3. Run `runyte` in the first directory: startup asks for a project directory.
4. Run `runyte --init .`: the editor opens instead of only initializing.
