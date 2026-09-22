# Windows support

Windows support is incomplete; Linux and macOS provide the full feature set.
The Phase-1 implementation and validation status are recorded below.
Runyte may omit or disable features on Windows when a sound implementation would
be disproportionately difficult. Platform-specific behavior must fail clearly
and leave user data intact. Native Windows access was available for the
2026-09-20 investigation below; continuing access for manual testing is not
assumed.

The initial scan examined `8910f13` (after the 0.3.0 release) on 2026-09-15
from Linux, using a Windows target check and a review of platform branches,
process spawning, and paths. The native follow-up examined `34d2378` on
2026-09-20. **Confirmed** means compiler, source, or specifically identified
native-probe evidence; it does not mean that the editor has run successfully.
**Likely** identifies behavior still needing an end-to-end reproduction.

## Delivery scope

The work may be delivered in two phases:

1. **Basic support:** standalone mode, no LSP, basic editing and file management,
   and integrated terminal splitting.
2. **Full or almost full support:** extend the usable standalone editor toward
   the Linux/macOS feature set, retaining explicit limitations where necessary.

Terminal splitting means independently running terminal sessions inside editor
panes. It requires ConPTY, input/output, resize, child exit, and cleanup in
phase 1; an editor with empty terminal panes or an unsupported-terminal error
does not meet that scope. LSP is deliberately unavailable in phase 1, including
when an existing configuration enables it. Syntax highlighting is independent
of LSP and can remain available.

The following allocation makes those boundaries concrete. Items not explicitly
included in basic support are proposed for phase 2, rather than implied phase-1
requirements:

| Area | Phase 1 | Phase 2 |
| --- | --- | --- |
| Editor | Open, edit, search, select, undo/redo, save, buffers and panes; Unicode and LF/CRLF handling | Remaining parity and refinements |
| File manager | Browse, create, rename, move, copy, confirmed deletion, collision refusal and recoverable failures | Broader filesystem and metadata support |
| Terminals | ConPTY shells in splits, focus, resize, review/scrollback, close and shutdown cleanup | Broader application compatibility and shell integration |
| Configuration/input | Windows configuration paths, usable keys, safe paste and text clipboard | Image paste and remaining integrations |
| LSP | Unavailable; no language-server processes | URI/path conversion, executable discovery, trust and lifecycle |
| Persistent sessions | Unavailable | Transport, private storage, attachment, discovery and lifecycle |
| Git, plugins, context bridge, shell filters | Proposed deferral, with clear unavailable states for anything omitted | Restore with their native dependencies and acceptance tests |
| `--wait`, `:quit-here`, system file/URL opening | May be deferred with documented errors | Standalone wait behavior, PowerShell integration and native openers |

Keep commands in the shared registry and make availability agree between
execution, help and the command palette. A runtime setting alone cannot remove
an unconditional reference to a Unix-only type: phase 1 still needs coherent
compile boundaries for deferred services and their frontend event channels.
The investigation below did not implement or disable features.

## Phase-1 implementation progress, 2026-09-20

Delivery order: complete and review Phase 1, then commit and push it to
`feat/windows-support`. Phase 2 begins immediately afterward and is divided
into sub-phases. The first, Phase 2.1, restores integrated Git. Git is optional
on Windows: check executable availability before enabling the integration, and
keep it disabled with a clear unavailable state when Git is not installed.
Missing Git must not cause repeated spawn failures or runtime errors. Each
implementation work package receives subagent review and incorporates its
actionable findings before the next package starts.

Phase 1 is implemented in six reviewed work packages on `feat/windows-support`,
after merging the 0.3.1 release from `main` (`e5d05da`) in `4bceec6`. Native
formatting, Clippy, the full workspace suite (2,601 passed, zero failures), the
optimized build and packaged executable smoke test pass. After CI fixture
repairs through `fc9c324`, remote run
[`35542444854`](https://github.com/runyte/runyte/actions/runs/35542444854)
passes every job, including native Windows tests, Linux/macOS regression suites,
and both 89% coverage gates. Detailed evidence is recorded in
`context/plans/completed/PLAN_WINDOWS_PHASE1.md`.

The selected target is `x86_64-pc-windows-msvc`, Windows 11 24H2 or later with
Windows Terminal. Native development uses Windows build `10.0.26200.9457`,
PowerShell `5.1.26100.9444`, cmd.exe and Rust 1.97.1. ARM64, MinGW, older
Windows, optional PowerShell 7/Git Bash and network shares remain unvalidated.

The implementation provides native configuration discovery, console input and
bounded text clipboard, Windows file identity and collision-safe filesystem
plans, and independently owned ConPTY terminal sessions. Deferred service
commands retain registry-backed unavailable states. LSP, integrated Git,
persistent sessions, plugins, context access, shell filters, image paste,
system opening, private diagnostic logs, `--wait` and `:quit-here` are unavailable
in Phase 1. Configuring a deferred service cannot start it.

A reported Phase-1 input problem is tracked separately in
[`windows_control_pane_keys.md`](resolved/windows_control_pane_keys.md): `Ctrl+h` in the
explorer was treated as Backspace, and `Ctrl+j` as Enter. Commit `af2218e` fixes
the native transport with reviewed decoder and real
ConPTY regressions; the resolution records the physical-capture limitation.

Terminal cwd must have a verified equivalent ordinary Windows spelling shorter
than 260 UTF-16 units. ConPTY stalled during the native extended-prefix cwd
probe, so long/verbatim-only terminal cwd is refused before process creation.
cmd.exe also refuses UNC cwd, since it would silently select another directory.
These terminal restrictions do not disable editing files through extended paths.
Filesystem plans reject cross-volume moves before mutation; network-share and
unsupported-filesystem behavior remains outside the validated support claim.

Native tests cover configuration precedence, key/text decoding, isolated native
clipboard round trips and bounded failure, locked/read-only saves, case-only
rename, hardlink/case/short-name collisions, rollback identity, shell quoting,
Unicode arguments/output, resize, independent shells, saturated output, child
and descendant cleanup, and failed starts. The real editor acceptance fixture
runs inside ConPTY with its own configuration and temporary files. Unix-only
service fixtures stay enabled on Unix, while unavailable-service expectations
and native path spellings are explicit on Windows.

The Linux/macOS 89% coverage gates remain unchanged. Native Windows coverage is
provisional without a measured llvm-cov baseline. Windows CI runs formatting,
all-target Clippy and tests; the release workflow adds an MSVC ZIP and includes
it in SHA256SUMS. No release publication or version bump is part of this work.

## Phase 2: native Git acceptance

Phase 2 begins with optional integrated Git. The local implementation now has
native executable discovery, isolated process ownership, strict repository
paths and editor availability. Git remains optional; absent Git does not start
a worker. The mixed-launch inheritance and delayed-reader failures have separate
regressions and reviewed corrections, and all 94 parallel provider tests pass.
The restored native editor/discovery suite passes 142 tests. Native handoff
passes formatting, all-target Clippy with warnings denied, and the full suite
(2,874 passed, zero failures, 34 ignored fixture/performance entries).
Cross-platform CI acceptance passes at `dbd30fc` in
[`35578396537`](https://github.com/runyte/runyte/actions/runs/35578396537),
including native Windows, Linux/macOS tests and both unchanged 89% coverage
gates. Sub-phase 2.1 is complete; the broader Phase 2 remains open.

Sub-phase 2.2 adds native private storage and standalone diagnostics on local
NTFS. Ownership, reparse refusal, pinned identity, atomic replacement and
cleanup have native coverage. Logs retain exclusive writer ownership across
rotation while remaining readable; real ConPTY acceptance covers `:log-open`,
default-log degradation and explicit-log refusal. All four packages have
independent reviews with no remaining findings. Native formatting, Clippy and
2,907 tests pass. All jobs in cross-platform acceptance run
[`35590400592`](https://github.com/runyte/runyte/actions/runs/35590400592) pass,
including both unchanged Unix coverage gates.

Sub-phase 2.3 restores language services after workspace permission, with native
executable discovery, owned asynchronous pipes/process trees, local-drive file
URIs and account-scoped permission storage. All three packages have independent
reviews with no remaining findings. Formatting, Clippy and the full native suite
pass (2,940 tests); required real rust-analyzer acceptance separately covers
initialization, diagnostics, edits, restart and revocation cleanup. CI provisions
and requires that acceptance. Cross-platform CI passes at `4d43fa6` in
[run 35606557698](https://github.com/runyte/runyte/actions/runs/35606557698);
standalone integrations, persistent sessions, plugins and context access remain.
Sub-phase 2.4's shell-filter package enables Windows PowerShell filters with
bounded UTF-8 streams, cancellation and owned process-tree cleanup. Independent
review has no remaining findings; formatting, Clippy, 2,957 native tests and
required real rust-analyzer acceptance pass. The next package restores image
paste through native PNG and DIB clipboard formats and private cache storage.
Its independent reviews have no remaining findings; native clipboard, cache
and editor tests, formatting, Clippy and the full suite (2,973 passed) succeed.
CI subsequently exposes shared window-station state between the text and image
fixtures. Reviewed repair `b26d65f` uses distinct create-only stations and a
coordinated isolation regression. Creating these stations requires privileges
unavailable to the local development token; three explicitly required Windows
CI tests provide that native acceptance, with no shared-clipboard fallback.
External opening is now implemented with nonblocking, bounded native dispatch,
literal file/URL arguments and program-cache updates after acceptance. Its
independent review has no remaining findings. Formatting, Clippy, 2,993 local
tests and required real rust-analyzer acceptance pass; privileged clipboard
and cross-platform opener acceptance remain pending. Standalone `--wait` and
PowerShell directory handoff are the following work packages.
Combined branch/worktree deletion is explicitly refused without mutation on
Windows: remove the worktree first, then delete its branch. Worktree switching
remains deferred with persistent sessions.
The ordered work packages, validation limits and continuation details are in
[`PLAN_WINDOWS_PHASE2.md`](../plans/active/PLAN_WINDOWS_PHASE2.md).

The independently validated Linux PTY allocation fix and resolution are
included as `8e2bd5d` and `c6ca884`, cherry-picked from `5e30ffb` and `6e6f270`
after integration review. Its regression is separate from the Windows process
inheritance regression. The remaining macOS gap is tracked in
[`macos_pty_descriptor_inheritance.md`](macos_pty_descriptor_inheritance.md).

The investigation below records the pre-implementation state. Its compiler
errors and "current" observations describe the inspected revisions, rather
than the working tree after the Phase-1 changes.
## Native evidence, 2026-09-20

Environment: Windows build `10.0.26200.0`, Windows PowerShell
`5.1.26100.9444`, Rust/Cargo `1.97.1`, host and installed target
`x86_64-pc-windows-msvc`, Git `2.55.0.windows.5`, and Visual Studio 2022
Community with the C++ tools component. `cl.exe` and `link.exe` were not on
the invoking shell's `PATH`, but Cargo's toolchain discovery found the native
tools and compiled the grammar dependencies. Absence from `Get-Command` alone
is therefore not evidence of a missing C toolchain.

| Check | Observed result |
| --- | --- |
| `cargo check --locked --target x86_64-pc-windows-msvc --lib --bins` | Dependencies compiled; Runyte library failed with 20 errors and two warnings. Binary checking was blocked by the library. |
| `cargo check --locked --target x86_64-pc-windows-msvc --all-targets --message-format short` | Library again failed with 20 errors; the library test build failed with 52 errors and four warnings. These counts overlap and are not 72 distinct defects. |
| Configuration environment | `HOME` and `XDG_CONFIG_HOME` absent; `USERPROFILE`, `APPDATA`, `LOCALAPPDATA` and `COMSPEC` present. The current default configuration resolver returns `None`. |
| Executable lookup | Joining `git` to each nonempty `PATH` entry found zero files; joining `git.exe` found one. `git --version` worked. |
| Native path spelling | A read-only `GetFinalPathNameByHandleW` probe of `README.md` returned a `\\?\` prefix. Reading the same file as `readme.MD` succeeded. |
| PowerShell helper input | A hidden child receiving UTF-8 bytes `63 61 66 C3 A9 0A` decoded the last two text characters as U+251C and U+00AE, instead of U+00E9. Its input and output encodings were both `ibm850`. |
| Checkout line endings | `core.autocrlf=true`; `git ls-files --eol` reported LF in the index and CRLF in the worktree for `Cargo.toml`, this issue, and `src/fixtures/stand-in`. |

The encoding probe used redirected raw stdin and an ASCII report of the
characters seen by `$input`, without reading or changing the system clipboard.
A separate echo probe changed the input's LF to CRLF and took about 1.4 seconds
for one PowerShell invocation; this is a single observation, not a benchmark.
An echo's output bytes can conceal incorrect decoding when the same code page
re-encodes them, so checking character values matters.

`WT_SESSION` was present, but the diagnostic shell's standard input and output
were redirected. No interactive Runyte input, clipboard-image, ConPTY, save,
or file-manager test passed or was claimed: the editor cannot yet compile.
No user clipboard, terminal configuration, or filesystem permissions were
changed. ARM64, GNU/MinGW, older Windows, network shares and other consoles
remain untested.

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

The original GNU cross-check reported 19 library errors. The native MSVC check
at `34d2378` reports 20, including one new type-inference error:

| Location | Missing on Windows |
| --- | --- |
| `src/plugin/process/runtime.rs` | `std::os::unix`, `crate::process_group`, `tokio::signal::unix`, `libc::pid_t`, `libc::SIGKILL`, `ExitStatus::signal`, `Command::process_group` |
| `src/workspace/host/plugin_handoffs.rs`, `src/plugin/handoff.rs` | `terminal::PendingTerminal`, `TerminalPreparation`, `TerminalCancellation`, `PENDING_TERMINAL_CHARGE`, `App::reserve_plugin_terminal`, `App::install_plugin_terminal` |
| `src/app/workspace_workflows.rs` (lines 929, 1534, 1544) | `crate::protocol`, which `src/lib.rs` compiles only on Unix |
| `src/app/git_workflows.rs:1350` | `branch_cascade_summary`, defined under `#[cfg(unix)]` |
| `src/lsp_trust.rs:35` | `let home = None` has no inferable inner type before `home.as_deref()` (`E0282`) |

More errors can emerge after these are fixed. In particular, source inspection
finds unguarded context-service integration in `src/main.rs`: `HostServices`
names `workspace::context::transport::Event`, `start_host_services` names
`storage::HostMode` and calls `start_context`, and the standalone loop calls
`context_delay`, `handle_context_event` and `sync_context`. Those modules and
host methods are Unix-only. These are additional source-confirmed binary
blockers, not part of the 20 compiler-reported library errors.

The native all-target check also reached library tests; see section 16.
The two ordinary-library warnings are unnecessary `mut` bindings in
`src/app/navigation_workflows.rs:124` and `src/fs_plan/staging.rs:66`.
They also need platform-aware treatment before warnings-as-errors Clippy passes.

The historical Linux check was reproduced without a Windows C toolchain.
Tree-sitter grammars compile C in their build scripts, so a stub compiler that writes empty object
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

Retain that recipe only as historical diagnostic evidence. It does not test
C compilation or linking. A native Windows CI job must check both the library
and binary, and build the tests; the native commands above now provide a
reproduction without compiler stubs.

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
Rust documents the `.exe` omission rule and the distinct risks of batch-file
argument decoding in [`Command`](https://doc.rust-lang.org/std/process/struct.Command.html).
Do not treat all `PATHEXT` entries as equally safe executables or introduce a
shell for ordinary executable-plus-argument-vector launches. Preserve the
existing exclusion of empty `PATH` entries. This boundary is needed for
phase-1 terminal programs even if Git, LSP and plugins are deferred.

## 3. Path representation

- **Language-server URIs are wrong (confirmed).** `lsp::path_to_uri` percent-
  encodes every byte outside `[A-Za-z0-9-._~/]`, so `C:\src\main.rs` becomes
  `file://C%3A%5Csrc%5Cmain.rs`. That puts the drive in the authority position
  and encodes the separators. The expected form is `file:///C:/src/main.rs`.
  At `34d2378`, `uri_to_path` decodes `file:///C:/src/main.rs` to
  `/C:/src/main.rs`, then rejects it with `Path::is_absolute()` because it
  lacks a Windows drive/UNC prefix, returning `None`. Diagnostics,
  go-to-definition, references, and other document requests need this conversion.
- **Verbatim prefixes (likely).** The project root, workspace identity, LSP
  trust records, and path containment all go through `fs::canonicalize`, about
  160 call sites in all. On Windows that returns `\\?\C:\...`. Such a path does
  not compare equal to or `starts_with` a non-canonical `C:\...`, so a check
  that canonicalizes only one side will refuse legitimate files. It also shows
  up in titles and messages. Git, language servers, and `cmd.exe` handle it
  inconsistently: `cmd.exe` does not accept a UNC working directory. A
  native handle-path probe above confirms the prefix on this machine; it does
  not establish which Runyte call sites fail. Keep canonical identity and
  containment separate from display and child-process path spelling. Blindly
  stripping prefixes changes extended-path semantics; test drive, UNC,
  verbatim-UNC and long paths at each boundary.
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
  Native lookup confirmed case aliases on this checkout, but duplicate-buffer
  behavior remains untested. Do not lowercase every path: support must account
  for case-sensitive directories as well as the default behavior.
- **Explorer entry names (likely).** `directory_buffer::parse_line` rejects
  only NUL, control characters, and newlines. On Windows a `\` in a typed name
  becomes a nested path. A `:` names an alternate data stream or a drive-
  relative prefix. `<>"|?*` are invalid, and names such as `CON`, `NUL`, and
  `COM1`, or names with a trailing dot or space, cannot be created normally.
  `fs_plan` already requires `Component::Normal` for plan paths, which catches
  prefixes, but the rest need a clear error before a plan is built.
- **Command-line parsers need their own Windows policy (confirmed).**
  `App::terminal_request` in `src/app/terminal_workflows.rs` uses POSIX
  `shlex::split`, which consumes unquoted backslashes: a command such as
  `C:\Tools\tool.exe` loses its separators. Merely replacing the PTY backend
  does not fix terminal launch syntax. Verify quoted executable paths, literal
  arguments, drive-relative `C:notes.txt`, rooted `\notes.txt`, spaces,
  non-ASCII names, and `gf` targets with `:line:column` suffixes. LSP URI fixes
  can wait for phase 2, but local editor and terminal path handling cannot.

## 4. Private runtime storage is a stub (confirmed)

The non-Unix `private_storage::platform::Directory` returns `Unsupported` for
every operation. The following features therefore fail on Windows:

- The diagnostic log, including `:log-open`.
- Pasted images (`Ctrl-v`). A PowerShell capture implementation exists, but the
  file cannot be stored; capture itself has not been exercised in this audit.
- Remembered LSP permission. The in-memory "Allow LSP once" path does not
  require this store, but LSP as a whole remains blocked by the other gaps.
- Private plugin state (`state.get` and `state.set` report
  `Private plugin state is unavailable on this platform`).
- Filesystem-plan sources retained through `OwnedFile`, which plugin staging
  uses.

The Unix implementation relies on descriptor-relative operations (`openat`,
`O_NOFOLLOW`, owner-only modes). A Windows equivalent needs owner-only DACLs,
refusal of reparse points, and handle-relative or re-verified opens. The
security policy lists several of these guarantees as in scope, so a weaker
Windows version must be documented as weaker.

Porting `Directory` alone is insufficient. `OwnedFile::create` in
`src/private_storage/owned_file.rs` unconditionally reads `/dev/urandom`, and
its non-Unix `same_file` always returns `false`. Provider staging and plugin
filesystem sources therefore also need native randomness and handle identity.
The buffer save code already uses `BCryptGenRandom`; it is a local precedent.
Phase 1 may defer image paste, plugin state and remembered LSP trust, but must
define what happens to logging and avoid enabling storage-dependent features
with an insecure fallback.

The log fallbacks also assume the worst case: `try_lock_exclusive` always
succeeds and `process_is_live` always returns `true`. Standalone logs from
exited processes would never be cleaned up, and two editors could rotate the
same file.

## 5. Filesystem plans lack required native operations (confirmed)

- `fs_plan::platform::rename_noreplace` returns `Unsupported` on everything
  except Linux and macOS, and `ApplyIo::rename` uses it for every step. Every
  explorer plan and plugin filesystem operation that renames therefore fails.
  `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` already refuses to
  overwrite, but the native implementation must preserve the plan's collision,
  cross-volume, rollback and recovery semantics; changing this one function
  is not sufficient to establish safe file management.
- `unix_fingerprint`, `buffer::file_identity` and
  `directory_listing::directory_identity` return `None` off Unix; staging's
  device and inode fields are compiled out.
  Conflict detection and external-change detection fall back to weaker
  evidence. The volume serial number and file index from
  `GetFileInformationByHandle` are the Windows equivalent.
  In particular, `fs_plan::staging::Identity` compares only entry kind on
  Windows. Cleanup can consequently accept a different file of the same kind
  as an owned staging entry. Native identity checks are a phase-1 prerequisite
  for enabling those cleanup paths, not an optional metadata improvement.
- Creating a symlink needs Developer Mode or an elevated process. Copying a
  symlink in a plan will fail for most users and should produce a clear error.
- `copy_file` copies read-only attributes through `set_permissions`. ACLs and
  alternate data streams are not preserved. That may be acceptable, but it
  should be a deliberate choice.
- The non-Unix buffer `sync_parent` is a no-op. This alone does not establish
  NTFS crash durability. Existing saves use `ReplaceFileW` with a recovery
  backup and reopen/sync the destination; new-file publication uses
  `MoveFileExW(MOVEFILE_WRITE_THROUGH)`. Validate these distinct paths,
  restoration failures, read-only files and files held without delete sharing.
  Microsoft documents partial failure states for
  [`ReplaceFileW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)
  and does not support its `REPLACEFILE_WRITE_THROUGH` flag. Keep durability
  claims narrower than the behavior actually tested.
- Recycle Bin behavior, junctions/reparse points, case-only renames, locked
  files, and cross-volume moves need explicit tests. Never substitute permanent
  deletion for a failed trash operation. Whether UNC/network and removable
  filesystems are supported in phase 1 remains a scope decision; unsupported
  operations must fail without losing the source or destination.

## 6. Integrated terminals

This section is required for phase 1 because terminal splitting is in scope.

- `TerminalManager::open` returns `Unsupported` off Unix, and `terminal/pty.rs`
  is Unix-only. A ConPTY backend is a second implementation of the hardest part
  of the terminal stack: spawning, resize, child exit, and draining output after
  exit. Plugin terminal handoffs depend on it too (see section 1).
- The default program should come from `COMSPEC`, or a configured shell such as
  `pwsh`, rather than a hard-coded `cmd.exe`. `pty.rs` sets
  `TERM=xterm-256color` and `COLORTERM`, which Windows console programs
  generally ignore.
- `local_hostname_is` always returns `false` off Unix, so OSC 7 reports with
  the machine hostname are rejected. Empty authority and `localhost` bypass
  that check, but a drive URI such as `file:///C:/...` still becomes a path
  rejected by `Path::is_absolute()`. Both hostname and path handling need a
  Windows implementation; do not assume every shell emits OSC 7 without setup.
- Terminal review of Codex and Claude Code relies on the child's escape
  sequences passing through ConPTY, which rewrites some of them (**likely**).
  The emulator's compatibility record was measured only against Unix PTYs.
- Keep the emulator and pane ownership shared, with a Windows backend beneath
  `src/terminal/`. ConPTY input/output must not block rendering; cover bounded
  queues, resize, EOF, output after child exit and cleanup after failed launch.
  Microsoft's [ConPTY lifecycle guide](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session)
  describes separate I/O servicing and final-output draining. Its
  [close API](https://learn.microsoft.com/en-us/windows/console/closepseudoconsole)
  changed behavior at Windows 11 24H2/build 26100; testing only this machine's
  build 26200 cannot validate shutdown on older supported builds.
- Distinguish closing/hiding a pane from terminating its terminal session.
  Verify horizontal and vertical splits with separate shells, repeated resize,
  focus changes, shell exit, explicit terminal close, and editor shutdown.
  Ordinary Windows console programs and VT applications both need coverage.

## 7. Process lifetime and cancellation

`process_group` is the only place Runyte signals a child tree, and it is
Unix-only.

Phase 1 already needs a sound terminal process-tree policy. Job Objects are a
candidate, not a drop-in Unix signal implementation: define child ownership
before execution, launch-failure cleanup, nested jobs, handle inheritance and
whether breakaway is allowed. Forced termination is distinct from delivering
Ctrl-C to the foreground console program. Microsoft's
[Job Objects documentation](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
describes job inheritance and termination constraints. Reuse that ownership
boundary for Git, LSP, plugins and filters as they become supported.

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

The non-Unix `pipe::invoke` bails with `shell pipes require Unix`, so
`:pipe <command>` / `:| <command>` do not work. The bare `|` key is reserved,
not an implemented filter binding. The Unix path runs `/bin/sh -c`.
Windows has no single equivalent: `cmd /C`, `pwsh -Command`, and Git for
Windows' `sh` differ in quoting and in what users expect. This needs a decision
and probably a configuration setting.

## 10. Opening files and links with the system (confirmed)

`OpenPlatform::CURRENT` is `Unsupported` off Linux and macOS, so on Windows:

- Binary files have no system default opener. The explicit program prompt still
  attempts a direct launch, but `external_open::launch` splits its program
  with `split_whitespace`: even `"C:\Program Files\Viewer\viewer.exe"`
  is split incorrectly. This is a separate confirmed parser gap, not just a
  missing system opener.
- `gf` on an `https://` or `www.` link fails with
  `System opener is unavailable`.
- Plugin requests to open a URL or file with the system fail the same way.

`ShellExecuteW` with the `open` verb takes the target as a separate argument
and avoids a shell. `cmd /c start` must not be used, because `&` and `^` in a
URL would be interpreted.

## 11. Clipboard (encoding gap confirmed natively)

Windows helper implementations exist, but their correctness is not established:

- Every yank and paste starts `powershell.exe`, which commonly takes several
  hundred milliseconds. The native diagnostic invocation took about 1.4 seconds;
  measure cold and warm editor operations separately before making a latency claim.
- Windows PowerShell 5.1 reads piped stdin and writes stdout in the console code
  page unless configured otherwise. The native `$input` probe confirmed that
  UTF-8 `café` becomes characters `c`, `a`, `f`, U+251C, U+00AE under CP850
  before `Set-Clipboard` would receive it. Set explicit encodings or use a
  native API; UTF-8 stdout must also be guaranteed for Runyte's decoder.
- `$input | Set-Clipboard` joins lines with CRLF and may append a trailing line
  break. `Get-Clipboard -Raw` returns CRLF text.

`windows-sys` is already a dependency, so the Win32 clipboard API
(`OpenClipboard`, `CF_UNICODETEXT`) would avoid the process, the encoding
problems, and the latency. Image capture could use `CF_DIB` the same way.
Text clipboard belongs in the basic editor acceptance checks; image capture
also depends on private storage and can be deferred. Actual clipboard tests
must preserve existing clipboard content and formats, and cover empty text,
non-ASCII text, multiline text, and exact trailing-newline behavior.

## 12. Keyboard, paste, and terminal input

- Crossterm's Windows backend reads console input records.
  `EnableBracketedPaste` returns `Unsupported` when it cannot use escape
  sequences, and `main.rs` treats that as a startup failure
  (`failed to enable bracketed paste`) on consoles without virtual-terminal
  support.
- **Confirmed dependency gap, interactive outcome untested:** locked Crossterm
  0.29.0's `event/source/windows.rs` reads console input records and creates
  key, mouse, resize and focus events. It has no `Event::Paste` path; bracketed
  paste decoding lives in `event/sys/unix/parse.rs`. Successfully emitting
  `EnableBracketedPaste` therefore does not establish a safe Windows paste
  route. In Normal mode, text arriving as keys can execute editor commands.
  This is a phase-1 acceptance blocker requiring an input design and a real
  Windows Terminal reproduction, including terminal paste shortcuts/right-click.
- Crossterm reports key release events on Windows. `main.rs` already ignores
  `KeyEventKind::Release`; confirm that repeat detection behaves. The locked
  parser emits Press/Release rather than Repeat and emits some Alt-code text
  on release, so blanket release filtering deserves an explicit regression test.
- Windows Terminal can intercept copy/paste, pane and `Alt` chords. Which keys
  reach Runyte depends on version and user settings; the previous report's
  blanket claim about default `Ctrl+C`/`Ctrl+V` bindings was not verified.
  Record tested bindings and the required configuration in the Windows guide.
- Keyboard enhancement flags are disabled off Unix. The console API reports
  modifiers directly, so this is probably correct, but distinguishing
  `Ctrl-i` from `Tab` and similar pairs needs checking.
- Include AltGr/non-US layouts, dead keys, IME/surrogate-pair text, mouse,
  focus and resizing. Restoration of console modes and the alternate screen
  must be tested after normal exit and startup failure. The current redirected
  diagnostic shell cannot provide this interactive evidence.

## 13. Configuration and per-user paths (confirmed)

- `config::default_config_root` uses `XDG_CONFIG_HOME` or `HOME/.config`.
  `HOME` is usually unset on Windows, so no user configuration is loaded. Theme
  and settings changes made from the editor then have nowhere to be saved.
  `%APPDATA%\runyte` is the conventional location.
  The missing `HOME`/XDG case was observed on the native machine, not merely
  inferred. Decide precedence for explicit `--config`, XDG overrides and the
  Windows default, and test configuration/theme saving with injected temporary
  locations. Do not assume PowerShell's `$HOME` variable is an environment
  variable inherited by Rust.
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
- The original scan found around 80 source and test files with `#[cfg(unix)]`
  code, and many tests
  run `sh`, `sleep`, or `/bin/sh`. `src/fixtures/stand-in` is a `/bin/sh`
  script installed through symlinks, which need privileges on Windows. The
  suite needs a Windows stand-in (a small compiled helper, for example) before
  `cargo test` can be meaningful there.
- The 2026-09-20 all-target check confirms test compilation failures in
  `src/external_open/tests/system.rs` (Unix imports, FIFOs and signals),
  `src/plugin/tests/process_runtime.rs` (signals/process APIs),
  `src/plugin/state/tests.rs` (Unix permissions, `StateLock`, `TestRuntimeRoot`),
  `src/app/tests/plugin_validation.rs` (`TestRuntimeRoot`),
  `src/app/tests/navigation_and_files.rs` and `src/app/tests/git.rs`
  (`crate::protocol`), and `src/app/tests/workspace.rs`
  (`parse_session_number`). Fix fixture boundaries as well as production
  compilation; do not count Unix-gated tests as Windows behavior coverage.
- `sh`, `bash`, and `pwsh` were not discoverable on this shell's `PATH` even
  though Git for Windows was installed. A native installation must not depend
  on incidental Git Bash utilities. The CRLF checkout of the checked-in
  `stand-in` script also needs consideration for any retained shell harness;
  do not change global Git settings to make tests pass.
- Build a native helper fixture through the build system if one is needed;
  do not write and execute ad hoc test scripts. Keep subprocess configuration
  and caches in fixture-owned temporary directories, including `XDG_CONFIG_HOME`
  for every editor/host launch. Run portable tests on Windows and retain
  Linux/macOS gates throughout the port.
- The coverage floor in `context/reference/test-coverage.md` applies per
  first-class target. Whether Windows becomes one is undecided.
  The current enforced floor is 89%; there is no Windows measurement yet.
  Basic support must have a named native build/test gate and a reported
  coverage result or an explicit provisional target status. Do not silently
  relax the existing floor or declare parity from a cross-check.

## 17. Agent context bridge (missing from the original audit)

The scoped context service is not the persistent-session frontend transport.
It can serve standalone workspaces on Unix too, so disabling persistent mode
does not automatically remove it from startup. Its Rust storage, discovery and
transport modules are Unix-gated; unguarded binary integration is listed in
section 1. Native commands such as `:context-access` need a clear unavailable
state in phase 1.

`bridges/runyte-context/runyte_context/client.py` imports `pwd`, obtains Unix
account IDs and uses `AF_UNIX` sockets and peer-credential checks. The bridge's
README explicitly requires Linux or macOS. Phase 2 requires a coordinated
editor/bridge port: authenticated local transport, account identity, private
grants, endpoint discovery and revocation, plus native conformance tests. A
socket-type substitution alone is insufficient. Keep the bounded protocol and
native approval boundary, and do not assume persistent sessions must ship
before standalone context access can be designed.

## Documentation to revisit when support lands

`README.md`, the installation section and terminal limitations in
`docs/user-guide.md`, `docs/faq.md`, `docs/plugins.md`, `SECURITY.md` (owner-
only log permissions, private storage, the persistent host), and
`CONTRIBUTING.md`. Also update `bridges/runyte-context/README.md` only when
the bridge is supported, and record actual Windows coverage and terminal
compatibility in their reference documents. Installation and release notes
must distinguish basic Windows support from full feature parity.

## Suggested order

1. Establish the phase-1 Windows target and explicit service availability.
   Restore library, binary and test compilation, including context-service
   references, and add native CI. Leave LSP and persistent sessions unavailable.
2. Establish safe outer-terminal input and text paste early (section 12),
   together with configuration paths and clipboard text. A runnable editor
   that interprets pasted text as commands is not ready for use.
3. Validate ordinary editing/saving and local paths. Port exclusive file-plan
   operations and staging identity together; cover rollback, trash and native
   sharing errors before enabling file-manager mutation.
4. Implement ConPTY, shell/program argument handling and terminal process
   ownership. Exercise split terminals, resize, review, child exit and editor
   shutdown. These are phase-1 work, not deferred parity items.
5. Complete the phase-1 acceptance matrix, supported-platform documentation and
   Windows packaging. Make every omitted command report its limitation.
6. In phase 2, add Git, LSP, plugins, shell filters and remaining integrations
   behind their own native acceptance tests. Private storage and process-tree
   support are shared prerequisites; LSP additionally needs URI conversion.
7. Design persistent-session and context-bridge transports with their identity,
   permission and lifecycle guarantees. Deliver either when complete, or keep
   the limitation explicit in an almost-full-support release.

## Phase-1 acceptance and remaining decisions

The first native milestone should target `x86_64-pc-windows-msvc`; this is the
configuration examined here, not a claim that Windows ARM64 or GNU/MinGW works.
The minimum Windows build and supported outer terminals remain undecided.
Windows 11 with Windows Terminal is a practical initial test configuration;
supporting older builds requires separate ConPTY/input tests. The default shell
and Windows command-line syntax also need an explicit decision; do not require
PowerShell 7 or Git Bash merely because they are convenient during development.

Acceptance should include the following small manual matrix, backed by native
automated tests for the noninteractive behavior. All data-changing checks use
temporary workspaces and test files:

| Check | Required result |
| --- | --- |
| Start and exit | Launch from PowerShell and cmd with default and explicit configuration; edit without LSP; quit normally and after a startup error with console state restored. |
| Editing and saving | Open/create/save/reopen Unicode and space-containing paths; preserve LF/CRLF in existing files, undo/redo and dirty-buffer protection; a locked/read-only destination gives an error with recoverable contents. Existing `preferred_line_ending` and CRLF-safe deletion code should be exercised, not replaced solely because the OS is Windows. |
| File manager | Browse, create, rename (including case-only), move, copy and delete to trash; collisions and stale plans refuse safely; injected mid-plan failure retains documented recovery data. Test native handle identity and reparse-point boundaries. |
| Input and clipboard | Non-ASCII and multiline text reaches the buffer unchanged; pasting in Normal mode never executes the pasted characters as editor commands. Verify both the editor's clipboard action and outer-terminal paste, key repeats, AltGr, mouse and resizing. |
| Split terminals | Run two independent shells in horizontal/vertical panes; type, focus, resize, scroll/review and return to live input. A busy child must not stall editing in another pane. |
| Terminal lifecycle | Shell exit drains output; hiding a pane preserves its terminal; explicit close and standalone editor shutdown clean up owned processes and handles, including descendants and failed starts. |
| Deferred features | Persistent-mode CLI/configuration and LSP requests fail clearly without starting services. Any deferred Git/plugin/context/filter command agrees with help and palette availability. |

Additional automated cases should cover reserved/device names, alternate data
streams, long and UNC paths, same-file aliases, cross-volume operations and
unsupported filesystems, according to the scope selected. Successful handling
or an intentional non-destructive refusal must be specified for each.

Before declaring phase 1 complete, run native `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, and the applicable
coverage gate, with Linux/macOS remaining green. `cargo check` is diagnostic
evidence only. Record the tested OS, architecture, terminal and shell versions,
and every intentionally unsupported feature; the eventual phase-2 label must
not erase those limitations without new evidence.
