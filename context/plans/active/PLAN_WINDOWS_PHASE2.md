# Windows Phase 2

## Checkpoint and next work package

Phase 1 is complete and pushed through `fc9c324`; remote acceptance run
[`35542444854`](https://github.com/runyte/runyte/actions/runs/35542444854)
passes every job. Sub-phase 2.1 now includes the native Git implementation:
executable discovery, isolated process ownership, native repository paths and
editor availability. Earlier prototype records are retained below as history.

Integrated Git is enabled when Git is installed. The process-ownership
correction passes all 94 parallel native provider tests, and restored editor
coverage passes 142 tests. Native handoff passes formatting, all-target Clippy
with warnings denied, and the full suite: 2,874 passed, zero failures and 34
ignored fixture/performance entries across 40 test binaries/doc-test groups.
Cross-platform CI acceptance remains pending; broader Phase 2 integrations
remain deferred.

The next package completes cross-platform acceptance. Combined
branch/worktree deletion is explicitly refused without mutation on Windows;
separate guarded worktree removal and branch deletion are supported. The
Unix session teardown coordinator remains unchanged. The missing-Git startup
acceptance passes with an isolated empty executable search path.

The Unix PTY investigation runs independently on `dev`, described by
[issue commit `9b265aa`](https://github.com/runyte/runyte/blob/9b265aa122dea9637023cf1f4549f5ce75a8639c/context/issues/unix_pty_descriptor_inheritance.md).
Bring its focused fix and regression commits into `feat/windows-support` by
cherry-pick after native Linux validation and review. The separate issue does
not establish a common cause for the Linux and Windows paths.

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

These packages are planned; native private storage and diagnostics are not
implemented by the Git sub-phase.

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
