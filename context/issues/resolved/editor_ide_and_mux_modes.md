---
title: "Launch modes are hard to tell apart and change by launch directory"
status: resolved
reported: 2026-10-05
resolved: 2026-10-05
commit: 2806b78
---

## Resolution

Fixed in `2806b78` ("Select editor, ide and mux modes by flag"), with the rest
of the change in `3bb65b6` ("Make --init only create a workspace"), `06933b2`
("Choose the default mode with a top-level mode setting"), `c363df8` ("Run as
runyte --editor when started as runed"), `cd01ab3` ("Describe the editor, ide
and mux modes in the docs"), `b3fe8d2` ("Close the remaining gaps around
editor, ide and mux modes") and `d705f0b` ("Keep the parent attachment request
Unix-only").

The launch path asked the launch directory which kind of session to run:
`launch_is_plain` in `src/main.rs` turned a target launch plain when discovery
found no workspace or the editor ran as root, and otherwise
`project_root::prompt` stopped startup to ask for a project directory. Both are
gone. `launch_is_editor` now answers from `--editor`, from the program name,
or from the configured `mode` when no mode option and nothing naming a
workspace (`--project-root`, `--init`) is given. Every other launch needs a
discovered workspace and fails with `project_root::NO_WORKSPACE_HERE` without
one. That covers a bare launch, a target launch, `-a` with no selector on Unix
and on both Windows paths, and a bare `-a` typed in an integrated terminal,
whose request `parent_attach_selector` now resolves to the shell directory's
workspace instead of sending the directory for the outer TUI to initialize.
Requests that name a directory (`-a DIRECTORY`, `:session-attach DIRECTORY`,
`Tab s`) still initialize it, as decided.

`--editor`, `--ide` and `--mux` (`-a`) replace `--plain`, `--standalone` and
`--persistent` in `src/launch.rs`, with no aliases. Editor mode is the former
plain session in `src/app/editor_mode.rs` with two changes: terminals are a
capability (`CommandCapability::Terminals`) refused with the editor-mode reason
and greyed in help and hints, and `open_terminal_at` refuses too so that
explorer and context actions cannot start one; and `:workspace-init`, the
mid-session service start in `main.rs` and the host identity refresh it needed
are removed, so a session never changes mode. The status line's `SessionMode`
words are `editor`, `ide` and `ide+mux`.

`--init` returns from `run` after `initialize_workspace` creates the state
directory and prints how to open it, before any terminal or editor exists, and
takes no mode option. The configuration gained a top-level `mode:` (`RunMode`
in `src/config.rs`) read through `Config::mode`, which falls back to the
deprecated `workspace.mode` (`LegacyWorkspaceMode`, mapped standalone to ide
and persistent to mux) and then to `ide`. `App::note_config_deprecations`
reports the old key at startup on Unix and Windows hosts and on
`:config-reload`; the settings page writes `mode`. The settings page cannot
remove a key, so a file that still has `workspace.mode` after `mode` is saved
is told the old key is ignored.

`refuse_workspace_modes_as_root` runs right after configuration loads, before
`-a DIRECTORY` could create anything, and lets only editor mode run as root;
`--wait`, `--serve` and `--init` are refused too. `runed` is the same binary:
`launch::started_as_runed` checks the program's file stem, and
`parse_as_runed` forces editor mode and rejects every option that would choose
another mode or a workspace; `runed mcp` opens a file, as
`runyte --editor mcp` does. `install.sh` creates `runed` as a relative link
beside `runyte` and leaves any other file of that name alone; release archives
carry a link on Linux and macOS and a copy, `runed.exe`, in the Windows zip.
In editor mode the tutorial skips its two terminal lessons and ends by
explaining the modes.

Deviation from the issue: when `mode: editor` is configured, `--project-root`
and `--init` still mean a workspace, because both name one; the issue did not
say.

Tests:

- `src/launch.rs`: `editor_ide_and_mux_select_the_launch_mode`,
  `runed_is_always_editor_mode`, `runed_is_recognized_by_its_file_stem`,
  `init_is_long_only_and_rejects_ambiguous_launches`.
- `src/main.rs`: `editor_mode_comes_from_the_flag_or_the_configured_default`,
  `root_runs_only_editor_mode`, `init_reports_a_new_and_an_existing_workspace`,
  `a_bare_parent_attachment_uses_the_existing_workspace_and_creates_none`,
  `editor_keeps_a_bare_launch_off_the_persistent_default`,
  `a_plain_host_starts_no_workspace_service`.
- `tests/editor_mode.rs`: real launches for the refusal outside a workspace,
  editor mode with and without a workspace and with `--log`, `--init`, a
  configured editor mode, `runed` through a symbolic link, and `runed mcp`.
- `src/app/tests/editor_mode.rs`: refusals including terminals, greying, and
  `no_path_starts_a_terminal_in_editor_mode`.
- `src/config.rs`: `workspace_state_defaults_and_accepts_the_original_root_spelling`
  covers `mode`, the alias and the warnings; `src/settings.rs`:
  `run_mode_persists_as_a_typed_unquoted_top_level_choice`;
  `src/app/tests/config_reload.rs`:
  `a_reload_reports_a_deprecated_mode_spelling_and_names_its_replacement`;
  `src/app/tests/presentation_and_settings.rs`:
  `run_mode_is_visible_and_saved_for_future_launches_only`.
- `src/app/tests/tutorial.rs`:
  `editor_mode_skips_the_terminal_lessons_and_ends_on_the_modes`.
- `src/ui.rs`: `the_status_row_names_the_workspace_mode_before_the_workspace`,
  `editor_mode_names_itself_in_the_rendered_status_row`.
- `tests/installer/test_install.py`: `test_runed_is_a_relative_link_to_runyte`,
  `test_an_existing_runed_that_is_not_the_link_is_left_alone`;
  `tests/release_packaging.rs` checks the archive link and copy.

Known limitation: `cargo install` installs only `runyte`; the user guide and
README show the link to add. The Windows zip's `runed.exe` is a full copy of
the executable, roughly doubling that archive.

## Report

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

### Expected behavior

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

#### editor

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

#### runed

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

#### ide and mux outside a workspace

`ide` and `mux` need a workspace: a Git root, or a `.runyte/` directory in
the launch directory or above it. Without one they refuse instead of asking:

```
no workspace here; run runyte --init DIRECTORY to create one
```

The interactive project-directory question is removed. The same refusal
applies to `-a`/`--mux` with no selector when the launch directory has no
workspace, which today makes the launch directory a workspace.

#### --init

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

#### Configuration

A top-level `mode: editor | ide | mux` setting selects the default mode, and
`ide` is the default when it is absent. `workspace.mode` is accepted as a
deprecated alias with its old values mapped (`standalone` to `ide`,
`persistent` to `mux`) and a warning naming the replacement. A command-line
mode flag overrides the setting.

#### Running as root

As root, only `editor` runs. `--ide`, `--mux`, and a configured `ide` or
`mux` mode refuse with a message pointing to `runed` and `sudoedit`, because
either would leave root-owned runtime state in a workspace another account
owns. This replaces the current rule that silently makes a root launch plain.
`--wait` already refuses as root.

#### Removed

- `--plain`, `--standalone` and `--persistent` (no deprecation period).
- `:workspace-init` and the mid-session workspace upgrade.
- Automatic plain sessions chosen by the launch directory.
- The interactive project-directory question at startup.
- `-a` with no selector creating a workspace in the launch directory.

`--wait` and `--serve` are unchanged and belong to `mux`.

#### Naming a directory still creates a workspace

The refusal above covers launches that rely on the launch directory: a bare
`runyte`, `runyte FILE` or `runyte DIRECTORY` in ide mode, and `-a`/`--mux`
with no selector. Requests that name a directory on purpose keep making it a
workspace when it is not one: `runyte -a DIRECTORY` (`--mux DIRECTORY`),
`:session-attach DIRECTORY`, and the explorer's `Tab s` ("open a persistent
session here").

### Constraints

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

### Reproduction

1. In a directory with no Git repository and no `.runyte/`, run
   `runyte notes.txt`: a plain session opens.
2. In a Git repository, run the same command: a workspace session opens.
3. Run `runyte` in the first directory: startup asks for a project directory.
4. Run `runyte --init .`: the editor opens instead of only initializing.
