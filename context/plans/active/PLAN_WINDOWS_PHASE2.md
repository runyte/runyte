# Windows Phase 2

## Checkpoint and next work package

Phase 1 is complete and pushed through `fc9c324`; remote acceptance run
[`35542444854`](https://github.com/runyte/runyte/actions/runs/35542444854)
passes every job. This documentation checkpoint does not include the unfinished
Phase-2 source. The package-1/2 implementation and partial package-3 results
below describe the existing local prototype, not code available from a fresh
checkout of this commit. Preserve that working tree when continuing; if it is
unavailable, these records describe the design to reconstruct and revalidate.

The prototype changes `src/git/cli.rs`, `src/lib.rs`, `src/windows_fs.rs`,
`src/terminal/pty_windows.rs`, `src/terminal/windows_command.rs`, and
`tests/git_provider.rs`, and adds `src/windows_process.rs`,
`src/git/executable_windows.rs`, and `src/git/paths_windows.rs`. Integrated Git
remains disabled. The parallel provider suite has unresolved failures.

The immediate package returns to the process-ownership boundary:

1. Add a deterministic native overlap test. Hold the custom launcher after it
   creates inheritable child pipe handles but before CreateProcess. During that
   interval, launch an unrelated compiled, long-lived fixture through ordinary
   `std::process::Command`. Use acknowledgments and bounded control channels to
   prove whether that sibling inherits a pipe and keeps it open after the
   intended Git child and its owned descendants exit. Identify the actual pipe,
   clean up all fixture processes on failure, and avoid timing sleeps or
   executing test-written programs.
2. Test delayed pipe-reader scheduling independently. An output-completion
   timeout is not itself proof of inheritance; keep the two diagnoses separate.
3. Request independent subagent review and incorporate findings before the
   correction package. Select the smallest reliable native ownership strategy
   from this evidence. An isolated helper parent and a shared native launch
   boundary are candidates; no helper design is committed by this record.
4. After the correction and its review, rerun the parallel real-repository
   provider suite, then finish path and editor workflows. Native branch deletion
   with a worktree must sequence removal before deletion; the existing non-Unix
   cascade path must not silently skip that step. Verify absent-Git behavior,
   full native gates and cross-platform CI before enabling integration.

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
