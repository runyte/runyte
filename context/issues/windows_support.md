# Windows support

Runyte does not run on Windows. Linux and macOS are the supported platforms.
Runyte may omit or disable features on Windows when a sound implementation would
be disproportionately difficult. Access to Windows for manual testing is
limited, so platform-specific behavior must fail clearly and leave user data
intact when it cannot be exercised regularly.

The findings below come from a scan of the tree at `8910f13` (after the 0.3.0
release) on 2026-09-15: a type-check against a Windows target plus a review of
every `#[cfg(unix)]`, `#[cfg(not(unix))]`, and target-specific branch, process
spawn, and path-handling site. The scan ran on Linux; nothing here has been
executed on Windows. Items marked **confirmed** follow directly from the code
or the compiler; items marked **likely** are expected from known platform
behavior and need verification on a Windows machine.

## Existing Windows code

Earlier work already covers part of the platform, and a port should build on it
rather than replace it:

- Saving uses `ReplaceFileW` and `MoveFileExW`, and new files are created with
  a private security descriptor (`src/buffer.rs`).
- Clipboard text goes through `powershell.exe` `Get-Clipboard` and
  `Set-Clipboard`, and image paste has a PowerShell capture script
  (`src/clipboard.rs`).
- The external-program cache uses `%LOCALAPPDATA%/runyte/cache`
  (`src/external_open.rs`).
- Filesystem plans create symlinks with `symlink_file` or `symlink_dir`
  (`src/fs_plan.rs`).
- `~` expansion falls back to `USERPROFILE`, and path completion accepts both
  separators (`src/app.rs`).
- A bare terminal request names `cmd.exe` (`src/app.rs`).
- Persistent-session commands stay in the command inventory and report
  themselves unavailable instead of being compiled out, through
  `reject_unavailable_persistent_session` in `src/app/input.rs`. `COMMANDS`
  feeds the command palette, help, and key dispatch from one table, and several
  tests pin its exact size, so a platform-conditional inventory would make those
  counts and `EditorCommand::ALL` differ per platform. Reporting unavailability
  through `CommandAvailability::Unavailable` tells a Windows user why a command
  cannot run instead of hiding it. Apply the same approach to any later feature
  that exists only on one platform.

Most other `#[cfg(not(unix))]` branches are stubs that return `Unsupported`,
`None`, or `true`. The sections below list the ones that matter.

## 1. The crate does not compile for Windows (confirmed)

`cargo check --target x86_64-pc-windows-gnu --lib --bins` reports 19 errors in
the library. Every one is a Unix-only item used from code that is not gated:

| Location | Missing on Windows |
| --- | --- |
| `src/plugin/process/runtime.rs` | `std::os::unix`, `crate::process_group`, `tokio::signal::unix`, `libc::pid_t`, `libc::SIGKILL`, `ExitStatus::signal`, `Command::process_group` |
| `src/workspace/host/plugin_handoffs.rs`, `src/plugin/handoff.rs` | `terminal::PendingTerminal`, `TerminalPreparation`, `TerminalCancellation`, `PENDING_TERMINAL_CHARGE`, `App::reserve_plugin_terminal`, `App::install_plugin_terminal` |
| `src/app/workspace_workflows.rs` (lines 929, 1534, 1544) | `crate::protocol`, which `src/lib.rs` compiles only on Unix |
| `src/app/git_workflows.rs:1350` | `branch_cascade_summary`, defined under `#[cfg(unix)]` |

Rustc stops at name resolution, so more errors are expected after these are
fixed. The binary, tests, and benches were not reached. The plugin process
runtime and plugin terminal handoffs are the most recent work, which suggests
the Windows build broke when they were added and nothing reported it.

The check was reproduced without a Windows C toolchain. Tree-sitter grammars
compile C in their build scripts, so a stub compiler that writes empty object
files lets `cargo check` reach the Rust code. Nothing is linked:

```sh
rustup target add x86_64-pc-windows-gnu
cat > "$STUB/cc" <<'EOF'
#!/bin/sh
out=""; prev=""
for a in "$@"; do
  [ "$prev" = "-o" ] && out="$a"
  case "$a" in -Fo*) out="${a#-Fo}";; esac
  prev="$a"
done
[ -n "$out" ] && mkdir -p "$(dirname "$out")" && : > "$out"
exit 0
EOF
chmod +x "$STUB/cc"
CC_x86_64_pc_windows_gnu="$STUB/cc" AR_x86_64_pc_windows_gnu=ar \
  CARGO_TARGET_DIR="$STUB/target" \
  cargo check --target x86_64-pc-windows-gnu --lib --bins
```

A CI job running this check, or a native `windows-latest` build, is the
cheapest protection against another silent break.

## 2. Executable discovery ignores `PATHEXT` (confirmed)

`service_health::resolve_configured_executable` joins the bare command name
onto each `PATH` directory and checks that the file exists. On Windows the file
is `git.exe`, not `git`, so `GitCli::discover` never finds Git and every Git
feature reports itself unavailable. The same resolver drives the service-health
report.

Spawning has a related gap. `std::process::Command` appends only `.exe` when it
searches `PATH`. Language servers installed through npm, such as
`typescript-language-server`, are `.cmd` shims. `lsp/transport.rs` would fail
to start them (**likely**), and so would `plugin/worker.rs` for any plugin
configured with a script or shim. Running a `.cmd` or `.bat` goes through
`cmd.exe` argument parsing, which Rust's standard library restricts, so the
resolution rule and its argument-safety limits need a design.

## 3. Path representation

- **Language-server URIs are wrong (confirmed).** `lsp::path_to_uri` percent-
  encodes every byte outside `[A-Za-z0-9-._~/]`, so `C:\src\main.rs` becomes
  `file://C%3A%5Csrc%5Cmain.rs`. That puts the drive in the authority position
  and encodes the separators. The expected form is `file:///C:/src/main.rs`.
  `uri_to_path` turns `file:///C:/src/main.rs` into `/C:/src/main.rs`, which
  is not a valid Windows path. Diagnostics, go-to-definition, references, and
  every other request that carries a document will fail.
- **Verbatim prefixes (likely).** The project root, workspace identity, LSP
  trust records, and path containment all go through `fs::canonicalize`, about
  160 call sites in all. On Windows that returns `\\?\C:\...`. Such a path does
  not compare equal to or `starts_with` a non-canonical `C:\...`, so a check
  that canonicalizes only one side will refuse legitimate files. It also shows
  up in titles and messages. Git, language servers, and `cmd.exe` handle it
  inconsistently: `cmd.exe` does not accept a UNC working directory. A
  simplified-path helper, which strips `\\?\` when the remaining path is still
  valid, is the usual answer.
- **Non-UTF-8 fallbacks are lossy (confirmed).** Off Unix,
  `workspace/identity.rs`, `lsp_trust.rs`, `git/status.rs`, and
  `git/worktree.rs` convert paths with `to_string_lossy` or reject non-UTF-8.
  Windows paths are UTF-16 and may contain unpaired surrogates. A lossy
  identity can let two distinct paths collide.
- **Case-insensitive names (likely).** NTFS compares names case-insensitively
  by default. The buffer list, Finder de-duplication, and path containment
  compare `Path` values exactly, so `Main.rs` and `main.rs` can open as two
  buffers backed by one file. macOS has the same property, so an existing
  decision may already apply.
- **Explorer entry names (likely).** `directory_buffer::parse_line` rejects
  only NUL, control characters, and newlines. On Windows a `\` in a typed name
  becomes a nested path. A `:` names an alternate data stream or a drive-
  relative prefix. `<>"|?*` are invalid, and names such as `CON`, `NUL`, and
  `COM1`, or names with a trailing dot or space, cannot be created normally.
  `fs_plan` already requires `Component::Normal` for plan paths, which catches
  prefixes, but the rest need a clear error before a plan is built.

## 4. Private runtime storage is a stub (confirmed)

The non-Unix `private_storage::platform::Directory` returns `Unsupported` for
every operation. The following features therefore fail on Windows:

- The diagnostic log, including `:log-open`.
- Pasted images (`Ctrl-v`). The PowerShell capture works, but the file cannot
  be stored.
- Remembered LSP permission. "Allow LSP once" still works.
- Private plugin state (`state.get` and `state.set` report
  `Private plugin state is unavailable on this platform`).
- Filesystem-plan sources retained through `OwnedFile`, which plugin staging
  uses.

The Unix implementation relies on descriptor-relative operations (`openat`,
`O_NOFOLLOW`, owner-only modes). A Windows equivalent needs owner-only DACLs,
refusal of reparse points, and handle-relative or re-verified opens. The
security policy lists several of these guarantees as in scope, so a weaker
Windows version must be documented as weaker.

The log fallbacks also assume the worst case: `try_lock_exclusive` always
succeeds and `process_is_live` always returns `true`. Standalone logs from
exited processes would never be cleaned up, and two editors could rotate the
same file.

## 5. Filesystem plans cannot be applied (confirmed)

- `fs_plan::platform::rename_noreplace` returns `Unsupported` on everything
  except Linux and macOS, and `ApplyIo::rename` uses it for every step. Every
  explorer plan and plugin filesystem operation that renames therefore fails.
  `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` already refuses to
  overwrite, so this is a small port.
- `unix_fingerprint`, `buffer::file_identity`, `staging` device and inode
  fields, and `directory_listing::directory_identity` are all `None` off Unix.
  Conflict detection and external-change detection fall back to weaker
  evidence. The volume serial number and file index from
  `GetFileInformationByHandle` are the Windows equivalent.
- Creating a symlink needs Developer Mode or an elevated process. Copying a
  symlink in a plan will fail for most users and should produce a clear error.
- `copy_file` copies read-only attributes through `set_permissions`. ACLs and
  alternate data streams are not preserved. That may be acceptable, but it
  should be a deliberate choice.
- `sync_parent` is a no-op, which is correct for NTFS but leaves durability
  claims in the user guide to be re-read for Windows.

## 6. Integrated terminals

- `TerminalManager::open` returns `Unsupported` off Unix, and `terminal/pty.rs`
  is Unix-only. A ConPTY backend is a second implementation of the hardest part
  of the terminal stack: spawning, resize, child exit, and draining output after
  exit. Plugin terminal handoffs depend on it too (see section 1).
- The default program should come from `COMSPEC`, or a configured shell such as
  `pwsh`, rather than a hard-coded `cmd.exe`. `pty.rs` sets
  `TERM=xterm-256color` and `COLORTERM`, which Windows console programs
  generally ignore.
- `local_hostname_is` always returns `false` off Unix, so OSC 7 working-
  directory reports are never accepted. Windows shells report `file://host/C:/...`,
  which needs the URI handling from section 3.
- Terminal review of Codex and Claude Code relies on the child's escape
  sequences passing through ConPTY, which rewrites some of them (**likely**).
  The emulator's compatibility record was measured only against Unix PTYs.

## 7. Process lifetime and cancellation

`process_group` is the only place Runyte signals a child tree, and it is
Unix-only.

- **Git (likely).** Cancelling a Git command kills only `git.exe`. Git for
  Windows starts `ssh`, credential helpers, and `git-remote-https` as further
  processes that inherit the output pipes. The reader threads in `git/cli.rs`
  would then wait until those exit. `finish_readers_or_stop` bounds the wait,
  but the remote operation keeps running. A Job Object with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` is the Windows equivalent of a process
  group.
- **Language servers (likely).** `kill_on_drop` kills the direct child. When
  that child is a `.cmd` shim, the `node` process it started keeps running.
- **Plugins.** `plugin/process/runtime.rs` waits for `SIGCHLD` and signals
  process groups. It needs a Windows child-exit path and tree termination.
- **Clipboard helpers.** The fallback calls `child.kill()` on the direct
  PowerShell process only.
- **Exit status.** `git/cli.rs::exit_signal` returns `None` off Unix. Messages
  that name a signal need a Windows wording that does not claim one.

## 8. Persistent sessions and `--wait`

- The host transport, endpoint discovery, and lifecycle use Unix sockets and
  Unix process semantics. `--persistent`, `-a`, `--session-list`, and
  `workspace.mode: persistent` all fail on Windows with
  `persistent mode is not yet supported on this platform`. It is acceptable to
  keep persistent sessions unavailable if the full lifecycle is not practical.
- **`--wait` is refused entirely (confirmed).** `main.rs` handles `Wait` in the
  same branch as `Persistent`, so `runyte --wait FILE` cannot be used as
  `GIT_EDITOR` or `EDITOR` on Windows. A standalone fallback that opens the
  files in a new editor and exits when that editor quits would cover
  `git commit` and agent prompt editing without a host.
- Recording recent workspaces, removing a worktree together with its session,
  and the branch-deletion cascade are compiled only on Unix. The session strip
  shows no targets.

## 9. Shell pipes are unavailable (confirmed)

The non-Unix `pipe::invoke` bails with `shell pipes require Unix`, so `|` and
the other selection filters do not work. The Unix path runs `/bin/sh -c`.
Windows has no single equivalent: `cmd /C`, `pwsh -Command`, and Git for
Windows' `sh` differ in quoting and in what users expect. This needs a decision
and probably a configuration setting.

## 10. Opening files and links with the system (confirmed)

`OpenPlatform::CURRENT` is `Unsupported` off Linux and macOS, so on Windows:

- Binary files have no system default opener. The explicit program prompt still
  works.
- `gf` on an `https://` or `www.` link fails with
  `System opener is unavailable`.
- Plugin requests to open a URL or file with the system fail the same way.

`ShellExecuteW` with the `open` verb takes the target as a separate argument
and avoids a shell. `cmd /c start` must not be used, because `&` and `^` in a
URL would be interpreted.

## 11. Clipboard (likely)

The PowerShell helpers work, but:

- Every yank and paste starts `powershell.exe`, which commonly takes several
  hundred milliseconds.
- Windows PowerShell 5.1 reads piped stdin and writes stdout in the console code
  page, not UTF-8. Non-ASCII text is likely to be corrupted in both directions
  unless the script sets `[Console]::InputEncoding` and `OutputEncoding`.
- `$input | Set-Clipboard` joins lines with CRLF and may append a trailing line
  break. `Get-Clipboard -Raw` returns CRLF text.

`windows-sys` is already a dependency, so the Win32 clipboard API
(`OpenClipboard`, `CF_UNICODETEXT`) would avoid the process, the encoding
problems, and the latency. Image capture could use `CF_DIB` the same way.

## 12. Keyboard, paste, and terminal input (likely)

- Crossterm's Windows backend reads console input records.
  `EnableBracketedPaste` returns `Unsupported` when it cannot use escape
  sequences, and `main.rs` treats that as a startup failure
  (`failed to enable bracketed paste`) on consoles without virtual-terminal
  support.
- Even in Windows Terminal, crossterm may deliver a paste as individual key
  events rather than `Event::Paste`. In Normal mode, pasted text would then run
  as commands. This is the highest-risk input behavior to verify first.
- Crossterm reports key release events on Windows. `main.rs` already ignores
  `KeyEventKind::Release`; confirm that repeat detection behaves.
- Windows Terminal binds `Ctrl+V`, `Ctrl+C`, `Ctrl+Shift+W`, and several
  `Alt` chords by default, so `Ctrl-v` image paste and some bindings never
  reach Runyte without changing the terminal's settings. The FAQ covers the
  equivalent macOS `Alt` problem and needs a Windows entry.
- Keyboard enhancement flags are disabled off Unix. The console API reports
  modifiers directly, so this is probably correct, but distinguishing
  `Ctrl-i` from `Tab` and similar pairs needs checking.

## 13. Configuration and per-user paths (confirmed)

- `config::default_config_root` uses `XDG_CONFIG_HOME` or `HOME/.config`.
  `HOME` is usually unset on Windows, so no user configuration is loaded. Theme
  and settings changes made from the editor then have nowhere to be saved.
  `%APPDATA%\runyte` is the conventional location.
- `user_paths` (the effective account's home directory) is Unix-only. The LSP
  trust directory and other owner-scoped state need a Windows source, such as
  `SHGetKnownFolderPath` or `%LOCALAPPDATA%`.
- The directory explorer's owner, group, and mode columns are empty off Unix.
  The Finder treats only dot-prefixed names as hidden (`file_picker.rs`), so
  Windows hidden and system attributes are not considered.

## 14. Shell integration

- The `:quit-here` wrapper in the user guide is Bash and Zsh only. PowerShell
  needs its own function around `--cwd-file`.
- `atomic_write_cwd_file` falls back to a plain `fs::write` off Unix.
- The agent prompt workflow (`EDITOR='runyte --wait'`) depends on section 8.

## 15. Plugins

Beyond the compile errors and private state:

- The plugin guide and examples assume scripts with a
  `#!/usr/bin/env python3` line and POSIX absolute paths
  (`args: [/absolute/path/to/...]`). Windows needs the interpreter spelled out,
  and `python3` is often the Microsoft Store alias stub rather than Python.
  ru-time has not been checked.
- The implementation stops and cancels plugins through process groups and
  signals, and needs a Windows counterpart (section 7).
- The conformance checks and `docs/plugins/check_*.py` harnesses have not been
  run on Windows.

## 16. Build, tests, CI, and release

- `ci.yml` builds and tests on `ubuntu-latest` and `macos-latest` only.
  `release.yml` produces four Linux and macOS archives with `SHA256SUMS`.
  Windows needs at least the check job from section 1, and eventually a `.zip`
  archive with checksums.
- `build.sh` is a Bash script. Installing from crates.io needs the MSVC or MinGW
  C toolchain for the grammars; this should be documented.
- Around 80 source and test files contain `#[cfg(unix)]` code, and many tests
  run `sh`, `sleep`, or `/bin/sh`. `src/fixtures/stand-in` is a `/bin/sh`
  script installed through symlinks, which need privileges on Windows. The
  suite needs a Windows stand-in (a small compiled helper, for example) before
  `cargo test` can be meaningful there.
- The coverage floor in `context/reference/test-coverage.md` applies per
  first-class target. Whether Windows becomes one is undecided.

## Documentation to revisit when support lands

`README.md`, the installation section and terminal limitations in
`docs/user-guide.md`, `docs/faq.md`, `docs/plugins.md`, `SECURITY.md` (owner-
only log permissions, private storage, the persistent host), and
`CONTRIBUTING.md`.

## Suggested order

1. Restore a compiling build and add the CI check (section 1).
2. Fix executable discovery, LSP URIs, and path prefixes (sections 2 and 3).
   With those, editing, Git, and language servers become usable.
3. Private storage, filesystem plans, configuration paths, and the system
   opener (sections 4, 5, 10, and 13).
4. Verify paste and input on Windows Terminal (section 12) before anyone is
   invited to use a Windows build.
5. Clipboard, shell pipes, and a standalone `--wait` (sections 8, 9, and 11).
6. ConPTY terminals, process trees, and plugins (sections 6, 7, and 15).
7. Persistent sessions, if they turn out to be practical at all.

Define a small manual smoke-test list that fits the limited Windows testing
available. Use automated tests for platform selection and for graceful
unsupported-feature errors wherever possible.
