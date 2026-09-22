# Windows Phase 2

## Checkpoint and next work package

Phase 1 is complete and pushed through `fc9c324`; remote acceptance run
[`35542444854`](https://github.com/runyte/runyte/actions/runs/35542444854)
passes every job. Sub-phase 2.1 is complete and includes the native Git implementation:
executable discovery, isolated process ownership, native repository paths and
editor availability. Earlier prototype records are retained below as history.

Integrated Git is enabled when Git is installed. The process-ownership
correction passes all 94 parallel native provider tests, and restored editor
coverage passes 142 tests. Native handoff passes formatting, all-target Clippy
with warnings denied, and the full suite: 2,874 passed, zero failures and 34
ignored fixture/performance entries across 40 test binaries/doc-test groups.
Cross-platform CI acceptance passes at `dbd30fc` in
[`35578396537`](https://github.com/runyte/runyte/actions/runs/35578396537),
including native Windows, Linux/macOS tests, lifecycle and plugin acceptance,
and both unchanged 89% coverage gates (91.92% lines on Linux, 91.83% on macOS).
Broader Phase 2 integrations remain deferred. The Git implementation is
`cdd3b8b`; all implementation packages received independent review and
incorporated their actionable findings before acceptance.

Sub-phase 2.2's private storage and diagnostics are complete. Sub-phase 2.3
restores language services with all three packages independently reviewed and
native acceptance complete; cross-platform CI acceptance passes at `4d43fa6`
in run `35606557698`. Sub-phase
2.4 restores the remaining standalone integrations. Combined
branch/worktree deletion is explicitly refused without mutation on Windows;
separate guarded worktree removal and branch deletion are supported. The
Unix session teardown coordinator remains unchanged. The missing-Git startup
acceptance passes with an isolated empty executable search path.

The independent Linux PTY fix and resolution from `5e30ffb` and `6e6f270` are
included here as `8e2bd5d` and `c6ca884` after integration review by `review_wp3`.
Their native Linux/macOS CI passed, including both unchanged coverage floors.
The Linux allocation race and Windows process inheritance failure have separate
regressions and fixes. The remaining macOS allocation window is recorded in
[`macos_pty_descriptor_inheritance.md`](../../issues/macos_pty_descriptor_inheritance.md).

## Phase-1 delivery history

Phase 1 was committed and pushed to `feat/windows-support` as `3203878`
(`Add native Windows Phase 1 support`). Its native suite passed 2,601 tests,
formatting and Clippy, with an optimized build and packaged executable smoke
test. The reviewed Linux lint correction followed as `c6fd317`.
Remote CI run `35539278558` passes Linux gates and both 89% coverage gates,
but its Windows test step and Linux MCP acceptance step failed. The supplied
failures led to reviewed fixture repairs in `3ce94ae`: deterministic directory
timestamps, same-volume native executable hardlinks, and a fresh-process PTY
helper for the threaded MCP harness. GitHub CLI now provides authenticated logs.
Run `35541629745` clears those failures and both Unix coverage gates, then
exposes a second cross-drive assumption in the headless cwd fixture. That
reviewed repair runs cwd changes in an isolated compiled subprocess.
It is pushed as `fc9c324`; remote run `35542444854` passes all jobs.

## Sub-phases

1. **Integrated Git.** Git remains optional. Discover a usable executable
   before enabling the integration; missing Git leaves commands and health in
   a clear disabled state without repeatedly attempting failed launches.
   Restore native paths, safe argument vectors, bounded output, cancellation
   and descendant cleanup before enabling editor workflows.
2. **Private runtime storage and diagnostics.** Native owner-only storage,
   reparse-point refusal, handle identity, randomness and locking establish the
   foundation for logs and durable service permissions.
3. **Language services.** Native process discovery/lifecycle, lossless document
   URI conversion and workspace permissions, with real server acceptance.
4. **Remaining standalone integrations.** Shell filters, image paste, system
   file/URL opening and standalone wait behavior; shell-directory handoff needs
   an explicit native shell contract.
5. **Persistent sessions.** Authenticated local transport, discovery, private
   runtime ownership, attachment and shutdown, preserving protocol bounds.
6. **Plugins and context access.** Native worker/process lifecycle, state,
   handoffs and context transport, preserving native approval ownership and
   validating the retained plugin contracts.

Each sub-phase is divided into reviewed work packages. Implement a package,
request an independent subagent review, incorporate actionable findings, then
start the next package. Dependency and acceptance details are refined before
each sub-phase begins. A deferred feature remains explicitly unavailable until
its native boundary is ready; the Phase-2 label alone does not enable it.

Local Windows validation is serialized with `CARGO_BUILD_JOBS=1` and
`RUST_TEST_THREADS=2`; agents do not run simultaneous builds or full suites.
This bounds memory pressure after a hard reboot during development. Memory
exhaustion has not been established as the cause of that reboot.

## Sub-phase 2.1 work packages

1. **Executable availability.** Injected PATH/PATHEXT lookup, absolute executable
   identity, no implicit current-directory search, missing-Git tests and health
   agreement. Keep the service disabled until its native execution boundary is
   ready.
2. **Native Git process ownership.** Own the process tree before execution,
   bound cancellation and inherited output pipes, preserve output-before-exit
   behavior and keep process work off the editor loop. Reuse the ConPTY job
   ownership boundary without changing terminal semantics.
3. **Native repository paths.** Separate canonical filesystem identity from
   Git directory argument spelling; preserve Unicode and literal pathspecs.
   Restore applicable real provider tests, including worktree operations and
   unsupported native directory refusal before side effects.
4. **Editor workflows and availability.** Enable Git only when available,
   restore editor tests, and handle standalone worktree removal sequencing.
   Unsupported cwd discovery errors stay latched until explicit refresh.
5. **Acceptance and documentation.** Absent Git, native status/diff/stage/commit,
   local remotes/worktrees, cancellation, full gates and Unix regression checks.
   Document native limits before declaring the sub-phase complete.

## Progress

### Native control-key correction before sub-phase 2.2

Real ConPTY capture reproduced the Ctrl+h/j loss at the VT conversion boundary.
The frontend now requests native keyboard reporting and decodes bounded wire
records before semantic keys or paste. Native Backspace/Enter retain their
actions, fast pane keys use the registry, and a supported configured pane alias
is covered. Encoded frames survive fragmented delivery without Escape timeouts;
raw paste remains literal even when its payload resembles a native frame.
`review_wp1` reviewed the reproduction and implementation, then repeated review
after both paste-safety findings were corrected. The final review has no
findings. Thirteen decoder/editor tests and two real ConPTY console tests pass.
Native formatting, all-target Clippy with warnings denied and the full suite
pass (2,881 tests, zero failures, 36 ignored fixture/performance entries).
The fix is `af2218e`, with its resolution recorded in `7524c5d`. All jobs pass
in [CI run 35584148523](https://github.com/runyte/runyte/actions/runs/35584148523),
including native Windows and both unchanged Unix coverage gates.

### Sub-phase 2.2 preparation

After Git acceptance, private storage and diagnostics proceed in four reviewed
packages:

1. Establish the native storage contract and regression fixtures: pinned
   directory identity, rejection of reparse points and hardlinked files,
   owner-only access, bounded reads, and behavior after names are replaced.
   Select a handle-relative native boundary from these tests; pathname
   validation alone is not an ownership boundary.
2. Implement the Windows `private_storage::Directory` interface and
   `OwnedFile` identity/cleanup. Preserve exclusive creation, nontruncating
   append, atomic replacement, and existing-versus-creating open semantics.
   Establish the native flush/rename guarantees explicitly before enabling
   callers that require durable storage.
3. Wire private diagnostic logs and their command availability to the verified
   storage boundary. Keep cache/configuration roots injectable and keep language
   permissions and other deferred integrations disabled until their own
   sub-phases validate them.
4. Validate replacement races, parallel writers, failure cleanup, bounded log
   behavior, native permissions, full handoff gates and cross-platform CI.
   Record any native durability limit rather than silently weakening the
   existing contract.

All four packages are reviewed with no remaining findings. Native formatting,
all-target Clippy with warnings denied and the full workspace suite pass:
2,907 passed, zero failures and 39 ignored fixture/performance entries across
41 test binaries/doc-test groups. Real-editor logging acceptance passes.
Cross-platform CI passes at `ccaeda6` in
[run 35590400592](https://github.com/runyte/runyte/actions/runs/35590400592).
Other dependent services remain disabled pending their own sub-phases.

Storage/diagnostics checkpoint `f0d7c8c` passes native Windows, ordinary Linux
and macOS suites, plugin/context acceptance and both unchanged coverage floors
in [run 35589383537](https://github.com/runyte/runyte/actions/runs/35589383537).
Its Linux lifecycle stress exposed an existing endpoint-directory cleanup race.
The independently reviewed correction `ccaeda6` moves directory preparation
under the stable identity lock and adds a deterministic retiring-host cleanup
regression. Its cross-platform run passes all jobs, including both unchanged
89% coverage gates. The Linux plugin-conformance job initially timed out during
the first Node registration; an isolated same-commit rerun passes unchanged.
The diagnostic gap and separate buffered-response test issue remain recorded
in `context/issues/node_conformance_readiness.md` for the plugin package.

#### Sub-phase 2.2 package 1: native storage contract

The selected boundary is `NtCreateFile` with a pinned parent directory handle,
one validated leaf component, synchronous access rights, no handle inheritance,
and reparse refusal plus handle metadata validation. A replaced directory name
must not redirect an operation. Paths are walked component by component; a
canonicalized pathname alone is not sufficient. Existing regular files need
one hard link and the current user's ownership before any write or ACL change.
New private files/directories receive a protected owner-only DACL at creation;
existing private leaves may be hardened only after identity and owner checks.
Read-only admission must not create entries or silently change permissions.

The implementation package must preserve exclusive creation, nontruncating
append, bounded reads, atomic same-directory replacement, and cleanup of only
the issued identity. Directory rename and replacement operate relative to the
pinned handle, not a reconstructed path. A published `OwnedFile` pathname must
still identify its issued file; replacements fail verification. Random names
use the system cryptographic generator. Windows byte-range locks establish
single-writer ownership for diagnostics before logging is enabled.

Durable operations flush file contents and directory metadata with the native
flush primitive. Contract acceptance distinguishes successful flush calls from
power-loss testing; it does not claim hardware behavior beyond the operating
system's contract. Initial private-storage support targets local NTFS with
persistent ACLs; unsupported filesystems must fail explicitly. No deferred
service is enabled by the contract probes.

`src/private_storage/windows_contract.rs` exercises pinned-directory replacement,
exclusive creation, invalid names, hardlink refusal, native directory flush,
junction refusal and the existing protected owner-only creation primitive.
The remaining operations receive behavior tests with the production interface
in package 2. `windows-sys` supplies native declarations through its existing
dependency; no new crate or minimum Rust version is introduced.

Native contracts: [directory-relative NtCreateFile](https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntcreatefile),
[metadata flushing](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers),
and [Windows file caching](https://learn.microsoft.com/en-us/windows/win32/fileio/file-caching).

#### Sub-phase 2.2 package 2: native storage implementation

The implementation uses handle-relative native opens and renames, checks the
pinned volume's remote-device flag, and validates current-user ownership before
permission hardening. Append handles request append access without write-data
access; replacement uses native POSIX semantics so existing readers retain the
old file. Cleanup deletes through an admitted identity and preserves substituted
entries. Read-only admission neither creates directories nor changes ACLs.

Review requested retrying failed ancestor flushes, remote-volume refusal,
post-creation failure cleanup, and replacement while a reader retains the old
file. Regression tests accompany these corrections. ACL helper review requested
no further changes after the ownership and access-mask tests were added.

Native probing confirmed that append access suffices for metadata flushing on
NTFS, but the ordinary development token cannot acquire it on the shared
`C:\Users` ancestor. `open_durable` therefore retains its full-ancestry contract
and propagates that refusal. `open_durable_beneath` instead takes an explicit
independently provisioned, already durable anchor and commits only its relative
subtree. It never infers an anchor from the nearest existing directory, creates
the anchor, or claims to persist the anchor's ancestry. It flushes the anchor
before mutation and each relative parent on every retry. Caller migrations,
especially plugin state, must establish that precondition before enablement.
Independent design review accepted this boundary without weakening the original
interface. Native [directory flush semantics](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/0de7dc40-9627-437e-a4df-c4696cdc3d02)
and [append-access flushing](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntflushbuffersfileex)
define the guarantee; a successful call is not a power-loss test.

Final review also corrected ACL propagation into existing children and explicit
creation ownership. Hardening uses the native per-object setter and changes no
descendant ACL. Creation names `TokenUser` explicitly rather than inheriting a
possibly group-valued default owner. Windows fixture roots use the same private
creation boundary. The descriptor regression passes; independent review has no
remaining findings. Formatting and full validation continue with diagnostics.

#### Sub-phase 2.2 package 3: diagnostics

Native logs use an exclusive lock outside their bounded content range, allowing
readers to open the log while its writer retains ownership. Rotation truncates
the issued file through a separately reopened native handle while preserving
the original locking handle. Process liveness is conservative on inaccessible
or unknown PIDs. Startup restores default logging and explicit `--log` behavior;
the normal registry enables `:log-open`. Native acceptance exercises real editor
startup, the log page, default degradation and explicit-destination refusal.
Independent review has no remaining findings. Fourteen logging tests pass;
two ignored compiled fixtures are exercised by the ownership and liveness
parents. Real ConPTY acceptance passes log opening, continued editing after
default-log refusal, and explicit-log startup failure. It also exposed a
workspace-root spelling bug: startup now canonicalizes the launch directory as
well as the requested root before checking containment. The fixture uses the
terminal emulator to inspect incremental screen updates.

### Sub-phase 2.3 preparation

Language services proceed in three reviewed packages:

1. Native executable discovery and process/transport ownership. Bare names use
   absolute PATH entries and native `.exe`/`.com` PATHEXT entries; explicit
   commands must be absolute native paths. Relative commands and implicit
   `.cmd`/`.bat` shells are refused. Script servers use an explicitly configured
   interpreter and argument vector. Discovery never executes a probe, and
   workspace permission precedes server execution. Preserve framing and stderr
   bounds, own pending pipe I/O, and test blocked stdin, saturated inboxes,
   leader-exit descendants, restart, connection drop and runtime shutdown.
2. Lossless Windows file URIs and exact-workspace permission storage. Define
   drive/extended-prefix/UNC handling and normalize server-returned identities;
   reject paths that cannot be represented without replacement characters.
   Trust identity uses exact native path encoding and account paths independent
   of inherited environment. Preserve project-local store rejection, remembered
   denial, one-time approval revocation and fail-closed storage errors.
3. Editor enablement, real rust-analyzer acceptance, documentation and full
   native/cross-platform gates. Required Windows CI provisions the server and
   exercises approval, initialization, diagnostics, edits, restart and cleanup.
   Other services keep their existing gates until their own sub-phases.

Independent preparation review accepted the decomposition and added the
executable, cancellation, URI identity and permission acceptance requirements
above. No user decision is needed for these implementation choices.

#### Sub-phase 2.3 package 1: native language-server processes

The process boundary reuses the owned native job and isolated inheritance
parent. LSP supplies private, overlapped named-pipe endpoints; no synchronous
pipe operation occupies a Tokio blocking worker. Pipe creation uses an
owner-only descriptor, a random name, first-instance admission, remote-client
refusal and verification that both initial endpoints belong to the launcher.
Connection drop aborts framing tasks even when their inbox is full. A guard
owned by the runtime stops the process tree when the runtime shuts down,
including before the monitor's first poll. Leader-exit observation terminates
descendants while allowing remaining stdout/stderr to drain.

Review removed an unnecessary blocking wait for the suspended inheritance
parent during launch. The job retains ownership through its termination;
Windows process handles need no Unix-style reap. Native acceptance passes six
lifecycle cases, eight process tests, eight framing tests and all 94 real Git
provider tests. Two executable-discovery tests also pass. Formatting and
denied-warning all-target Clippy pass. Final independent review has no remaining
findings. Editor language services remain disabled until URI and trust acceptance.

#### Sub-phase 2.3 package 2: document identity and workspace permissions

Windows document URIs accept absolute drive paths and their equivalent
extended-prefix spelling, including long paths and Unicode. Conversion refuses
UNC/device namespaces, remote authorities, alternate data streams, unpaired
UTF-16 and names that need verbatim-only interpretation. Raw escapes and decoded
components are checked before URL normalization. Server-returned locations are
matched to existing buffer identities while accepting the response, so picker
previews retain unsaved text without reading file contents. Diagnostics and
edits retain their existing version and containment checks.

Account profile/cache roots come from known-folder records, not inherited
environment defaults. Scoped COM initialization preserves existing apartments
and balances only its own successful initialization. Trust identity uses exact
canonical UTF-16LE bytes. The home-workspace exception takes an independently
resolved standard cache, including redirection; arbitrary workspace-local
overrides remain rejected even with several nonexistent path components.
Storage still admits the original path through its reparse/ownership boundary.

Forty-three LSP tests, one native live-buffer identity test, seven trust tests,
one COM lifecycle test and one cache mapping test pass. Formatting and
denied-warning all-target Clippy pass. URI/identity and account reviews have no
remaining findings after the COM prerequisite correction and its regressions.

#### Sub-phase 2.3 package 3: editor enablement and real-server acceptance

The Windows manager now retains configured LSP
enablement, and standalone startup loads exact-workspace permission before
attaching it. Command dispatch, help, hints and health use the ordinary manager
and document capabilities. Native executable discovery and permission remain
separate: approval is required before attempting a server launch, and a missing
server leaves editing available.

One hundred restored editor language tests, five native platform-boundary tests
and real ConPTY paste/save acceptance pass. A separately required native
rust-analyzer test passes permission denial/approval, initialization, syntax
diagnostics, formatting edits, restart, revocation and process-tree cleanup.
The same compiled fixture drives the real editor's permission chooser and
missing-server error through ConPTY. Its config, cache, Cargo home and target
directories are temporary, and native process handles observe only descendants
of its enclosing fixture job. CI provisions the server and checks that the
explicitly selected acceptance actually ran.

Independent final review has no remaining findings after documenting Windows
config paths, script interpreter configuration and the shared process cwd
limit. Formatting, denied-warning all-target Clippy and the full native suite
pass: 2,940 passed, zero failures and 41 ignored entries across 42 libtest/doc
groups, plus six harness-free native transport cases. The final required real
server acceptance passes separately in 1.62 seconds with isolated Cargo home.
Cross-platform run
[`35593594927`](https://github.com/runyte/runyte/actions/runs/35593594927) passes
all seventeen non-Windows jobs, including both unchanged coverage floors.
The Windows suite finds a setup-prompt fixture assumption: the longer runner
path wraps `[y/N]:` across display rows. Correction `4d43fa6` matches setup
prompts across rows while retaining row boundaries for the editor screen.
It chooses terminal width to force the regression without lengthening cwd.
Independent review has no remaining findings, and native acceptance passes.
Full cross-platform acceptance passes in
[`35606557698`](https://github.com/runyte/runyte/actions/runs/35606557698).

### Sub-phase 2.4 work packages

1. Native shell filters: preserve admission limits, asynchronous host ownership,
   exact UTF-8 output, atomic selection replacement, cancellation and cleanup.
2. Native image clipboard formats and private image-cache storage, with isolated
   clipboard fixtures and restored editor image-paste tests.
3. Native system file/URL opening and explicit program selection, retaining
   literal target arguments and detached ownership appropriate to GUI programs.
4. Standalone `--wait` behavior and an explicit PowerShell directory-handoff
   wrapper for `:quit-here`, with native editor and shell acceptance.

Each package receives independent review until no findings remain before the
next implementation begins. Sub-phase 2.3 checkpoint `9aa3a18` is pushed;
the reviewed prompt-fixture correction and current acceptance run are recorded
above.

#### Sub-phase 2.4 package 1 contract

Windows filters use the installed system Windows PowerShell with no profile and
noninteractive text I/O, independently of the terminal's `COMSPEC`. A fixed
encoded bootstrap reads the user command from a child-only environment entry,
removes that entry, and invokes the script block. Selected text stays on stdin.
This preserves the existing 16 KiB command limit: encoding the entire command
could exceed the native command-line bound, and cmd has its own smaller bound.
Console input/output and native-pipeline output encoding use UTF-8 without BOM.
Commands can read exact input through `[Console]::In.ReadToEnd()`; ordinary
PowerShell object pipelines retain their own formatting behavior.

The process boundary reuses native jobs and isolated inherited handles, with
the audited overlapped-pipe constructor extracted from LSP. One current-thread
runtime in the existing background filter worker owns raw pipe I/O; framing
remains specific to LSP. The job-wide deadline, aggregate stdout limit, bounded
stderr, cancellation and leader-exit descendant cleanup remain required.
Review specifically requires successful native stderr and failing native/cmdlet
commands to be distinguished before adopting an error-action policy.

The design review found no ownership blocker. Invocation and command limits
follow Microsoft's [Windows PowerShell invocation contract](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_powershell_exe?view=powershell-5.1)
and [cmd command-line limit](https://learn.microsoft.com/en-us/troubleshoot/windows-client/shell-experience/command-line-string-limitation).
Implementation and independent review are complete, with no remaining findings.
Review added native regressions for invalid UTF-8 output, stderr flooding,
blocked stdin and descendant-held output after leader exit. Eight backend
tests, nine host workflow tests, five platform boundary tests and real ConPTY
filter/undo acceptance pass. The shared pipe extraction also retains LSP's
framing and lifecycle tests. Formatting, all-target Clippy with warnings denied
and the complete native suite pass: 2,957 passed, zero failures and 42 ignored
entries across 42 libtest/doc groups, plus six harness-free transport cases.
Required real rust-analyzer acceptance passes separately after the extraction.

Checkpoint `275a650` passes the Linux CI gates, but native run `35608605740`
reports eight PowerShell-dependent filter test timeouts and a separate existing
log rotation acknowledgement timeout. The direct compiled-fixture pipe tests
pass. This does not establish RAM exhaustion, shell startup latency or a pipe
deadlock. Windows CI now uses the locally validated one-build/two-test-thread
resource envelope; explicit concurrency tests retain their own overlapping
children. Independent review has no findings. Product and fixture deadlines
are unchanged. Native Windows acceptance passes at `6dd14ef` in run
`35609722577`; that run's sole failure is a separate Linux MCP fixture readiness
race, corrected with independent review in `fd1cd7b` and recorded in
[`mcp_workspace_discovery_readiness.md`](../../issues/resolved/mcp_workspace_discovery_readiness.md).

The ConPTY save fixture now waits for the editor's successful write status
before reading completed contents. A single pre-completion `NotFound` read is
recorded separately in
[`windows_save_path_visibility.md`](../../issues/windows_save_path_visibility.md);
neither a persistent save failure nor its underlying cause is established.

#### Sub-phase 2.4 package 2 contract

Image paste retains text priority when the clipboard advertises plain text,
including Windows-synthesized Unicode text. Otherwise it prefers registered
PNG data, then DIBV5 and DIB bitmap data. Native reads share the existing one
worker and one-second editor wait; bounded data is copied before the clipboard
is released and conversion begins. PNG bytes pass through without decoding.
Bitmap conversion bounds input, decoded RGBA dimensions and encoded output
independently, each to 64 MiB, and streams rows into PNG encoding.

Supported bitmap layouts have validated INFO/V4/V5 headers, palettes or
nonoverlapping contiguous bit masks, aligned rows and explicit orientation.
Unused BGRX bytes are opaque; alpha requires an explicit mask. Unsupported
compression or color profiles are refused, never followed as filesystem paths.
Advertised image read/conversion failures remain errors rather than text paste.
Native fixtures use a private window station and explicitly associate worker
threads with its desktop; they never replace the user's clipboard.

The contract follows Microsoft's
[clipboard format conversion](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats)
and [bitmap header](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-bitmapv5header)
documentation. Two independent design reviews found no blocker; implementation
and independent implementation review are now complete with no findings.
Thirteen native clipboard tests, nine cache tests and 98 editing workflow tests
pass. The native fixture reacquires clipboard handles after opening instead of
retaining handles transferred to Windows; sequence and repeated-byte checks
verify reads do not replace clipboard content. Formatting, all-target Clippy
and the complete native suite pass: 2,973 passed, zero failures and 42 ignored
entries across 42 libtest/doc groups, plus six harness-free transport cases.
Cross-platform acceptance remains pending.

#### Sub-phase 2.4 package 3 contract

Default file/directory/HTTP(S) opening uses ShellExecuteEx on a bounded STA
worker, suppressing ordinary shell error dialogs; Windows security prompts are
not suppressed. NOASYNC requests worker-side file dispatch without relying on
a message loop, but Windows does not apply that flag to URI/namespace items.
The editor must observe dispatch acceptance asynchronously rather than
wait for shell association lookup. A successful dispatch is not proof that the
target application displayed the file. Unknown startup outcomes are never
automatically retried, and outstanding calls retain admission slots.

Explicit program choices use native executable discovery and Windows argument
parsing, append the literal target as one argument, and launch without inherited
handles or a kill-on-close job. Process/thread handles close after accepted
creation; no worker waits for the viewer to exit. This permits the application
to outlive the editor, subject to any externally imposed job restrictions.
Default choices remain display labels rather than fabricated executables.
Existing foreground-authority checks and validated plugin targets are retained.

Tests must inject the shell association boundary instead of opening the user's
browser or viewer or changing associations. Compiled native fixtures verify
arguments, handle isolation and survival after the launching helper exits.
The contract follows Microsoft's
[ShellExecuteEx](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecuteexw)
and [CreateProcess](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw)
contracts. Two design reviews identify no decision blocker. Editor ports return
accepted or pending dispatch, and cache updates require confirmed acceptance.
The existing maintenance tick drains completions without adding an idle timer.
Implementation follows the reviewed and natively validated package 2.

Package 3 implementation has independent review with no remaining findings.
All 21 backend tests and five editor dispatch tests pass, including a compiled
viewer that preserves literal arguments and survives its launcher without
inheriting its handles. Formatting, all-target Clippy and full native acceptance
pass: 2,993 tests, zero failures and 46 ignored fixture/performance/privileged
acceptance entries across 42 libtest/doc groups, plus six harness-free native
transport cases. Required real rust-analyzer acceptance passes in 3.96 seconds
after the shared executable-resolver extraction. Remote opener acceptance is
pending; the independently reviewed clipboard repair is pushed as `b26d65f`
and its required privileged acceptance runs in `35615440695`.

Package 2's CI run `35612547548` passes all Unix jobs and both coverage gates,
including the repaired MCP discovery scenarios. Windows exposes a clipboard
fixture isolation error: `CreateWindowStationW(NULL, 0, ...)` can reopen the
same logon-derived station in the text and image fixture children. Independent
investigation confirms the API contract; a unique create-only station and a
coordinated two-process regression replace that assumption. Native probes
confirm this development token cannot create a named station even with minimal
rights and default security; create-only unnamed creation also finds an existing
station. There is no safe shared-station fallback. The three native fixtures
therefore run as explicit required acceptance in the administrator-capable
Windows CI job, which checks each passing result. They are ignored in ordinary
local tests; conversion and worker tests remain enabled. Product clipboard
behavior and its privileges are unchanged. This follows the documented
[station creation restrictions](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createwindowstationw)
and [hosted runner privileges](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).

#### Sub-phase 2.4 package 4 contract

Implement and review standalone wait behavior before the PowerShell handoff.
On Windows, `--wait` opens a new standalone editor and returns when that editor
quits. It retains parser target requirements and normal save/discard checks;
closing one requested buffer does not complete the invocation. Startup or
terminal loss must return failure. Unix attachment and wait-token semantics
remain unchanged until native persistent sessions are implemented separately.

The PowerShell wrapper targets Windows PowerShell 5.1. It creates private
handoff storage, invokes the native editor with literal argument framing,
consumes a bounded lossless UTF-16 record only after successful exit, changes
directory with `Set-Location -LiteralPath`, and cleans up in `finally` without
exiting the caller's shell. The editor pins an already-private parent before
editing and writes atomically through the native storage boundary. Existing
App quit safety and pane-directory selection are reused. Unix handoff bytes
and writing remain unchanged. Compiled fixtures and checked-in shell scripts
must cover argument fidelity, path characters, refused/forced quit, failure
exit codes, unchanged cwd on failure and cleanup. Extended path compatibility
must be established by native acceptance rather than blind prefix removal.

### Sub-phase 2.5 preparation

Read-only dependency review proposes six packages; persistent availability
remains disabled until native acceptance completes:

1. Lossless Windows protocol paths and workspace identity, with malformed
   path-byte refusal, unpaired UTF-16 coverage and unchanged Unix encoding.
2. Private endpoint publication, registry locks and pinned process identity.
   Separate transport addresses from filesystem metadata and expose probes
   instead of treating named-pipe addresses as ordinary socket files.
3. Native named-pipe accept/connect and authenticated peers beneath the shared
   bounded framing, response queues and generic `serve_connection`. Keep the
   buffered client's reader independent of rendering, with bounded cancellation.
4. Detached host ownership, supervision, conservative stale-record recovery,
   stop/restart and catalog state. Detached hosts outlive launchers while
   foreground/test-supervised hosts retain their shutdown obligations.
5. Attached frontend, persistent wait and workspace switching. Parent-terminal
   capabilities also require native ConPTY job membership and pinned peer
   identity; process ancestry snapshots alone are insufficient.
6. Native lifecycle/attachment acceptance and registry-backed availability:
   dirty/terminal/wait shutdown guards, duplicate launches, restart races,
   single interactive ownership, handshake ordering, partial/stalled frames,
   namespace isolation and cross-terminal authorization refusal.

The protocol already supports generic asynchronous streams, so a second
Windows protocol implementation is unnecessary. Windows persisted-path
encoding, process/boot identity and detached ownership transfer are refined
and reviewed in their owning packages. Dependency review finds no user
decision blocker.

### Current implementation evidence

Editor availability now depends on native executable discovery instead of a
Windows-wide Git exclusion. Missing Git creates no Git service; commands,
palette and health retain their ordinary absent-executable state. A compiled
main-binary fixture invokes real `start_host_services` with empty child PATH
and fixture-owned configuration, verifies no Git event receiver is created,
and checks four Git commands remain unavailable without an editor exit.
The test passes. The shared editor/discovery suite passes 142 native tests.
`review_wp1` requested correction of the acceptance fixture's command API and
explicit ConPTY cleanup in two restored editor tests; both were incorporated.
Combined branch/worktree deletion now refuses instead of queuing branch
deletion while skipping its worktree. This limit and optional-Git behavior are
documented in the user guide and keymap register.

Final repository-path review found a separate lossy Windows worktree decoder.
Porcelain now rejects invalid UTF-8; malformed `.git` links yield absent facts
and malformed `commondir` files retain the private Git directory as fallback.
Unix preserves raw path bytes. The native regression creates an actual U+FFFD
directory and verifies malformed bytes cannot redirect any of those reads into
it. All ten metadata/parser tests pass. This incorporates `review_wp2`'s final
required finding before editor enablement.

The process correction uses an isolated inheritance parent created suspended
with no inherited handles and atomic job membership. It never executes
application code. Local pipe handles are noninheritable; their inheritable
duplicates exist only in that parent's handle table. The actual Git child uses
`PROC_THREAD_ATTRIBUTE_PARENT_PROCESS` and an explicit handle list, inheriting
the same owned job before execution. The temporary parent is terminated after
creation or on setup failure. This follows
[Microsoft's isolated-parent approach](https://devblogs.microsoft.com/oldnewthing/20260511-00/?p=112313)
without adding a helper executable or a runtime helper protocol. A separate
suspended process is created per command; its cost is included in native
provider acceptance, not a claimed startup benchmark.

Windows Git output uses PeekNamedPipe to read only available bytes, then a final
drain after leader exit and job termination. Reader scheduling no longer has a
100 ms success deadline. The overlap regression now requires EOF while the
unrelated child is still alive, and verifies it could not inject its marker.
Eight native process tests pass (one ignored compiled fixture is exercised by
them), including failure/unwind cleanup and job-close termination of the
suspended parent. The gated output regression passes, and all 94 real provider
tests pass concurrently in 42.82 seconds. `review_wp3` reviewed both packages;
its bounded-read, live-sibling and finalizer-comment findings were incorporated.
Final native handoff gates pass after the editor-workflow changes. The full
suite also caught a stale Phase-1 test expecting every Windows Git command to
be platform-disabled; it now leaves Git availability to the missing-executable
acceptance fixture, which checks the exact nonfatal command outcome.

The continuation diagnostics on 2026-09-21 confirm two separate problems.
`mixed_standard_spawn_exposes_the_custom_pipe_inheritance_window` held the
native launcher before CreateProcess, launched an unrelated compiled fixture
through `std::process::Command`, and received that fixture's marker through the
exact stdout pipe after the intended child and job exited. The marker read uses
PeekNamedPipe and a bounded available-byte read; it does not wait for global EOF.
`delayed_reader_deadline_is_not_evidence_of_inherited_pipes` held a reader behind
an explicit gate until the existing completion deadline rejected it, then
released it and recovered complete Git output and EOF. Both native diagnostics
passed. This establishes inheritance and a separate scheduling false positive;
it does not assign every prior provider failure to either cause.

### Historical prototype checkpoint

The entries below predate the corrections and acceptance results above.
Their disabled-Git and failing-provider statements describe that earlier
checkpoint, not the current implementation or next actions.

Sub-phase 2.1 package 1 is implemented. `review_wp2` found no blocking issue
and requested precise documentation of the public discovery method's ambient
PATHEXT read; that documentation was corrected. The internal native resolver
injects both values and only accepts .exe/.com candidates in absolute PATH
directories. Three native tests pass for missing Git, PATH/PATHEXT ordering,
Unicode paths, wrapper rejection and malformed extensions. No candidate is
executed during lookup. Production Git remains deferred until package 4;
package 2 now establishes native process ownership.

Package 2 introduces native job ownership shared with ConPTY. Git commands use
CreateProcessW with a job list assigned before execution, a three-pipe handle
allowlist, direct UTF-16 arguments and captured environment overrides. Job
closure or cancellation stops descendants, including after their leader exits.
The existing Git worker retains its output and timeout bounds. Unix process
groups are unchanged, and no new dependency or minimum Rust version is needed.

`review_wp3` found no required correction. Its two additional coverage
suggestions were incorporated: exclusion of an unrelated inheritable event,
and cancellation of blocked stdin/stdout workers. Native compiled fixtures also
cover arguments/environment, stdin, exit status, failed launches and descendant
cleanup after leader exit, cancellation and drop. An available real Git is
exercised for version output, output-size refusal and hash-object input. The
production integration remains disabled pending package 4 and acceptance.

The full native suite exposed startup stalls in the new compiled process
fixtures under the normal token despite passing focused sandbox-token tests.
The normal-token failure reproduced independently with the extended-prefix cwd;
using its identity-equivalent ordinary spelling makes all six process tests
pass in 0.35 seconds. The exact Windows startup mechanism was not diagnosed.
The existing terminal conversion now lives in `windows_fs` and is shared with
Git; it rejects cwd names without an ordinary equivalent shorter than 260
UTF-16 units. `review_wp3` reviewed this extraction and requested that package 3
document and preflight this limit. Ordinary editor file I/O is unaffected.

Final package-2 validation on the native Windows development host passes:
`cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
and `cargo test --locked --workspace --no-fail-fast`. The complete rerun has
2,611 passing tests, zero failures and 32 ignored fixture/performance entries
across 40 test binaries/doc-test groups. Compiled fixture entries are exercised
by their nonignored parent tests. The reviewed package is ready for package 3;
the reviewed Phase-1 repair rerun also passes all 2,611 native tests. Remote
validation proceeds through authenticated GitHub CLI checks.

Package 3 is partially implemented: discovery and worktree records use native
canonical identities, while worktree directory operands use verified ordinary
spellings. Unsupported destinations are refused before branch creation.
Real provider tests are restored on Windows with ordinary fixture arguments,
explicit LF configuration (including the initial clone checkout), and a legal
Windows bracket pathspec fixture. Production availability remains disabled.

The first parallel native provider run passed 65 of 93 tests. Most failures
reported an inherited pipe still open after Git exited; the same operations
passed in focused runs. `review_wp3` confirmed a process-ownership gap against
Rust 1.88's standard Windows launcher: it serializes inheritable-handle creation
and CreateProcess with an internal lock, while Runyte's custom native launcher
creates inheritable pipe handles outside that lock. Its explicit handle list
limits what its own child inherits, but another concurrent standard launcher
can still inherit those handles. Killing the Git job cannot close a handle in
an unrelated process. A longer reader timeout does not establish ownership.

This is a design blocker for enabling native Git. Package 2's focused process
tests did not exercise overlap with the standard launcher. The native process
boundary and its mixed-launch regression coverage must be resolved before
package 3 can pass parallel acceptance or package 4 enables editor workflows.
The separate initial-clone CRLF and invalid Windows filename fixture failures
are corrected; the full provider suite is not claimed green.

Partial package-3 review by `review_wp2` found that an offline worktree root
could fail the entire list. Missing rows now retain their reported spelling if
their existing prefix cannot be resolved, without relaxing mutation preflight.
Focused coverage includes unavailable roots, canonical Unicode identities,
missing descendants, and invalid ordinary directory operands. Real provider
coverage also checks that invalid destinations never create the requested ref.

The process audit confirms the structural inheritance gap, not which individual
test errors were leaks rather than late reader scheduling. Current ConPTY
launches disable handle inheritance; other Windows product services remain
deferred. Concurrent standard spawns occur in the real-repository fixtures and
can also come from an embedding host or later services. Stable Rust 1.97.1 still
has the private lock; custom process attributes remain nightly-only, so an MSRV
bump alone is insufficient. The design choices are a shared native launch
boundary covering product and fixture spawns, or an isolated helper parent
whose inheritable handles never exist in the embedding process. A deterministic
mixed-launch overlap test must accompany that decision; increasing the output
grace period alone is not a fix.

`review_wp2` accepted the offline-root correction with no further findings.
The three native path unit tests and the real invalid-destination/ref-integrity
test pass, as do final formatting and all-target Clippy. The new package-3
source remains uncommitted with the earlier Phase-2 work; only reviewed Phase-1
code has been pushed. The continuation package is specified at the top of this
plan, with integrated Git still disabled and parallel provider acceptance
unresolved.
