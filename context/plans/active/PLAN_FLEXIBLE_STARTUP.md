# Flexible startup

Status: approved, not started. Tracking issue:
[#71](https://github.com/runyte/runyte/issues/71). Work items:
[#72](https://github.com/runyte/runyte/issues/72),
[#73](https://github.com/runyte/runyte/issues/73),
[#74](https://github.com/runyte/runyte/issues/74),
[#75](https://github.com/runyte/runyte/issues/75),
[#76](https://github.com/runyte/runyte/issues/76).

Status and discussion live on GitHub. This plan holds the design, the order of
work and the checks an implementation must pass. The decisions and their
rejected alternatives are recorded in #71; this plan does not repeat the
arguments, only the outcome.

## Goal

Runyte starts usefully wherever it is launched: a graphical launcher (macOS
`Runyte.app`, the Linux desktop entry), a terminal in the home directory, or a
project. It starts in the configured `mode`, offers integrated terminals
wherever a workspace exists, and always says which workspace it chose or, when
it refuses, why and what to run instead.

## Outcome in one table

| Launch | Today | After this plan |
|---|---|---|
| Finder, Dock, application menu | editor mode in `/` (macOS) or the launcher's directory | the configured mode in `~`; ide/mux use the home workspace |
| `cd ~ && runyte` (ide) | refuses: home is not adopted | the home workspace |
| `cd ~ && runyte --mux` | refuses: no workspace | the home workspace, starting or attaching its session |
| `runyte --init ~`, `runyte -a ~` | creates `~/.runyte/`, which every later launch below `~` discovers | the home workspace; nothing is written into `~` |
| `cd ~/scratch && runyte` (no repository) | adopts `~/scratch`, creating `~/scratch/.runyte/` | unchanged |
| No usable workspace | `no workspace here; run runyte --init DIRECTORY to create one` | names the mode, where it looked, why, and commands that work from here |
| Any ide/mux startup | status line shows the path only | the interaction line also says how the workspace was found |

## Invariants

- Discovery markers are `.git`, `.jj`, `.svn` and the relative state directory
  (`.runyte/` by default). No project marker files.
- The home workspace applies only when the launch directory is exactly a home
  directory and discovery found nothing. Subdirectories never discover it.
- Runyte never creates a new state directory inside the home directory. An
  existing `~/.runyte/`, or a checkout whose work tree is `~`, still wins.
- The filesystem root, the temporary directory, and shared or foreign-owned
  directories are never adopted automatically.
- A running session never changes mode.
- Editor mode and `runed` are unchanged. Running as root still allows only
  editor mode.
- Tests never resolve real per-user configuration, cache or state
  directories; every environment-derived path is injectable.

## Design

### Shared data model

Two values carry this plan through startup. Introduce them first, in #75 or
#76, whichever lands first, so the later items only add variants.

```rust
/// How startup arrived at the workspace it opens.
pub enum WorkspaceOrigin {
    VersionControl(VersionControl), // Git, Jujutsu, Subversion
    StateDirectory,                 // an existing relative state directory
    Home,                           // #73
    Created(CreatedBy),             // ide adoption, --init, -a DIRECTORY
}

/// Where a workspace keeps its runtime state.
pub enum StateLocation {
    InTree(PathBuf),    // <root>/<workspace.state>, or an absolute workspace.state
    OutOfTree(PathBuf), // <per-user state root>/workspaces/<id>   (#72)
}
```

- `project_root::discover` returns the root together with its
  `WorkspaceOrigin` rather than `Option<PathBuf>`. It already knows which
  check matched, so this adds no filesystem work.
- `project_root::resolve_state_root` (or its successor) returns a
  `StateLocation`. It is the only place that decides in-tree versus
  out-of-tree. Logs, plugin downloads, pasted images, host names and the host
  endpoint keep calling it and need no other change.
- The refusal message (#75) is built from the mode, its source (flag or
  setting), the launch directory and the adoption reason, replacing
  `project_root::NO_WORKSPACE_HERE`.

### #75 Clear refusals (independent)

- Replace `NO_WORKSPACE_HERE` and the text built in `project_root::adopt` with
  one function. It must cover each site listed in the issue: the bare-`-a`
  path, the mux path, the parent-context attach, `parent_attach_selector`, and
  `App::new_with_targets`.
- Ways out differ by mode, because an ide launch cannot name a workspace
  elsewhere: `--project-root` is internal and requires the launch directory to
  be inside it.
  - **mux**: `runyte --mux DIRECTORY`, `runyte --init DIRECTORY`.
  - **ide**: `cd DIRECTORY && runyte`, `runyte --init DIRECTORY`,
    `runyte --editor`.
- Fix `--init`'s "open it with" hint, which suggests `runyte <root>` although
  that does not open `<root>` from another directory.
- Add a fixture test that runs each suggested command from the launch
  directory.

### #76 Startup echo (independent)

- Map each `WorkspaceOrigin` to one line, using the table in #76, and emit it
  through the action-echo path (`ActionFeedback`), so it persists until
  replaced.
- **Ordering**: compute the echo last during startup, after the root,
  deprecation, logging and service notices. Set it only if nothing else has
  claimed the interaction line. Add a single "startup interaction line owner"
  check rather than per-message special cases.
- **mux**: the host's editor state produces the line. Emit it on host start,
  on attach, and on a `WorkspaceSwitchRequest` completing. Do not emit it on
  an automatic reconnect to the same host.
- The adoption notification stays as a retained entry; the echo is the
  visible message.

### #72 Out-of-tree state (foundation)

- **Per-user state root**:
  - Linux and other Unix: `$XDG_STATE_HOME/runyte`, else
    `~/.local/state/runyte`.
  - macOS: `~/Library/Application Support/runyte`.
  - Windows: `%LOCALAPPDATA%\runyte`.

  On Unix, take the home directory from the effective account, as the
  platform cache directory does (see `user_paths::system_home_directory`).
  Resolve it lazily: launches that never use out-of-tree state must not pay
  for it.
- **Per-workspace directory**: `<state root>/workspaces/<workspace id>/`, with
  the same layout as `.runyte/`. Create it owner-only through
  `private_storage`. Refuse a state root that another account owns or can
  write to.
- **Reserved roots**: add the state root to `reserved_user_roots`, so no
  in-tree state may overlap it, and check it does not overlap the
  configuration or cache roots.
- **Host endpoint length**: without `XDG_RUNTIME_DIR`, the usual case on
  macOS, the endpoint falls back to `<state>/host/…`, and `sun_path` is 104
  bytes there. Compute the worst case for
  `/Users/<name>/Library/Application Support/runyte/workspaces/<32 hex>/host/…`.
  If it can exceed the limit, give out-of-tree workspaces a short per-user
  runtime endpoint directory. Do not shorten the workspace ID; MCP and
  discovery depend on it.
- **Windows**: check the interaction with `workspace.state_anchor`, which
  bounds durable plugin state, and with `host-names/`.
- **Environment fingerprint**: context discovery matches hosts by
  `XDG_RUNTIME_DIR` and `XDG_CACHE_HOME`. Decide whether the state root
  belongs in that fingerprint, so test hosts with a temporary state root stay
  hidden from real ones.

Implementation handoff for #73: `project_root::resolve_state_location` is the
single placement decision; `resolve_state_root` is its path-only compatibility
wrapper. `workspace_state::StateLocation` retains placement for endpoint and
Windows durability decisions. Add home selection at that central decision,
then route discovery/initialization through it. Do not infer placement from an
absolute `workspace.state` path.

Debug subprocess fixtures use `test_support::user_environment` (or
`isolate_user_paths`) to inject configuration, cache, state, runtime, context,
account home and Windows account LocalAppData. The fixture-only
`RUNYTE_TEST_OUT_OF_TREE_ROOT` selects one exact canonical project; paired
`RUNYTE_TEST_STATE_HOME` provides the final Runyte state root on every platform,
including macOS. `RUNYTE_TEST_ACCOUNT_HOME` is read before account caching.
Release builds ignore these selectors. Keep the ordinary production resolver
and consumers in the test path; no process-global environment mutation is
needed. An ignored macOS `XDG_STATE_HOME` alone is insufficient isolation.

### #73 Home workspace (needs #72)

- **Resolution order** in the launch path (`src/cli.rs`):
  1. `discover` finds a version-control root, or an existing relative state
     directory: use it.
  2. Nothing found, and the canonical launch directory equals a home from
     `user_home_directories()`: the home workspace, with
     `StateLocation::OutOfTree`. This applies to both ide and mux; mux may
     create it.
  3. Nothing found elsewhere: an ide launch adopts as today, minus the home
     refusal; mux refuses as in #75.
- **Explicit routes**: `--init ~`, `-a ~`, `--mux ~`, `:session-attach ~` and
  `Tab s` on `~` go through the same check, so they reach the same workspace
  ID and never create `~/.runyte/`. The check belongs in `initialize`, or in a
  wrapper every route calls, so no route bypasses it.
- **Contained scans**: the file picker, the project finder and workspace grep
  use the contained boundary (`scan_boundary`) when the workspace origin is
  `Home`, and skip `~/Library` on macOS. The skip is a fixed exclusion for the
  home workspace only, not a general ignore rule.
- **Root launches**: `refuse_workspace_modes_as_root` still runs first, so
  root never reaches the home workspace.

### #74 Graphical launchers (needs #73)

- **macOS**: `RunyteLauncher` sets its working directory to the account's
  home directory before `exec`, drops `--editor`, and keeps stripping a leading
  `-psn_*` argument.
- **Linux**: the desktop entry becomes `Exec=<runyte> --window %F`. Make the
  start directory explicit: preferably a `Path=` line written by the
  registration script, which already writes absolute paths.
- **File arguments**: a graphical launch with files discovers its workspace
  from the first file's directory, not the launch directory, so opening a
  project file from a file manager uses that project. This changes only
  graphical launches: the launcher passes a flag naming the launch as
  graphical, and the terminal behavior is untouched. Confirm the rule on #74
  before implementing it.
- **Visible failure**: with the home workspace a bare graphical launch should
  not refuse. List the remaining refusals (root, an unusable state directory,
  an unparsable configuration) and make the window open and show them. Do not
  add a pre-launch dialog; it was rejected in #71.

## Order of work and pull requests

1. #75 and #76, in either order, each its own PR. Whichever lands first
   introduces `WorkspaceOrigin`.
2. #72, a PR with no user-visible change apart from the documentation of the
   state root.
3. #73, which adds `WorkspaceOrigin::Home` and its echo row in #76's table.
4. #74, which needs manual acceptance on macOS before the issue closes.

#69 (the macOS Dock icon for terminal launches) is independent and can land
at any point.

## Testing

- **Isolation**: inject the home directory, the state root, the runtime
  directory and the configuration path in every test. Subprocess fixtures set
  `XDG_CONFIG_HOME`, and from #72 a fixture-owned state location. No test may
  resolve the real ones (`AGENTS.md`, Working conventions).
- **Executables**: never run an executable a test wrote. Use
  `src/fixtures/stand-in` where a fixture needs a program.
- **Platforms**: cover Linux and macOS in CI. Include the socket-length test
  on macOS, and Windows for the state root and `state_anchor`.
- **Coverage**: keep line coverage at or above the floor in
  `context/reference/test-coverage.md`.
- **Startup cost**: record the effect of discovery and state-root changes in
  `context/reference/startup-performance.md` if it is measurable.

## Documentation to update

- `docs/user-guide/workspaces-and-persistent-sessions.md`: finding the
  workspace, the home workspace, the refused list, the startup echo.
- `docs/user-guide/configuration.md`: where workspace state lives, and the
  home-directory warning.
- `docs/user-guide/install-and-run.md`: launch forms and refusals.
- `INSTALLATION.md`, `README.md` and `contrib/packaging/README.md`:
  graphical launches no longer open editor mode.
- `context/issues/resolved/editor_ide_and_mux_modes.md`: a dated later-change
  note for the home workspace, keeping the original record.

## Open questions, settled on the issue before implementing

- #72: whether the state root joins the context-discovery environment
  fingerprint.
- #72: where the endpoint goes if the macOS path can exceed `sun_path`.
- #74: `Path=` in the desktop entry versus setting the directory at launch.
- #74: the file-argument rule for graphical launches.
- #74: how the remaining graphical-launch refusals are shown.
