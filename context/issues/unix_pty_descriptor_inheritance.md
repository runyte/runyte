# Unix PTY descriptors may be inherited by unrelated child processes

## Evidence and expected behavior

Source inspection on 2026-09-21 at `dev` commit `5e8a436` found a potential
descriptor-inheritance race in `src/terminal/pty.rs::open_pair`. The function
calls `libc::openpty`, takes ownership of the returned master and slave, then
sets `FD_CLOEXEC` on each endpoint with separate `fcntl` calls. The source does
not establish atomic close-on-exec creation. The effective flags returned by
the supported platform implementations and an actual overlapping launch need
native verification; no Linux runtime failure has been reproduced yet.

If either endpoint is initially inheritable, an unrelated process launched
between creation and the later `fcntl` can retain that endpoint after exec.
Changing the parent's descriptor flags afterward does not change the child's
copy. A retained endpoint can extend PTY lifetime or affect terminal shutdown
and EOF behavior. Such symptoms are possible consequences, not established
diagnoses of a previously observed Linux test failure.

An unrelated executed program must not retain either endpoint of a Runyte
terminal session. The intended terminal child must still receive its standard
input, output and error, controlling terminal, process group and initial size.
Ordinary Git subprocess pipes on Linux already use Rust's atomic
`pipe2(O_CLOEXEC)` path; the PTY allocation boundary needs separate inspection.
Close-on-exec does not prevent temporary descriptor inheritance between fork
and exec, and this issue does not promise isolation from a child that forks
and deliberately never executes another program.

## Native reproduction and regression boundary

The first work package establishes a deterministic Linux reproduction before
selecting a fix:

1. Inspect the flags returned for both endpoints and add a test-only
   synchronization point around the allocation boundary. Hold the vulnerable
   implementation after `openpty` returns and before either close-on-exec
   update, without exposing such a hook through the production public API.
2. While allocation is held, launch an unrelated compiled fixture through
   `std::process::Command`. The fixture must actually exec. Check whether it
   retained the particular PTY endpoints, using endpoint identity rather than
   only descriptor numbers, which can be reused by the fixture's runtime.
   On Linux, PTY device identity may require more than `fstat` alone.
3. Return the observation through an explicitly owned control channel and
   release the allocation barrier. Use bounded waits, explicit acknowledgments
   and cleanup on every failure path. Fixed sleeps and probabilistic stress
   loops are not the primary reproduction.
4. Demonstrate the failure against the vulnerable implementation, then retain
   a regression that forces the equivalent concurrent-launch opportunity at
   allocation in the corrected implementation and proves neither endpoint
   survives exec. Also exercise allocation failure cleanup and preserve normal
   terminal input, output, resize and shutdown behavior.

Fixtures use a compiled test executable or an existing checked-in executable;
they must never execute files written by the test. Storage and configuration
belong to temporary fixture directories. Every editor or host subprocess sets
fixture-owned `XDG_CONFIG_HOME`, and tests must not change process-global
environment variables in a concurrent test binary. Fixture control descriptors
must themselves have deliberate inheritance and close-on-exec ownership.

## Implementation scope and constraints

The second work package closes the demonstrated allocation race with the
smallest reliable native boundary. An atomic close-on-exec allocation path is
preferred where supported. Setting the flag earlier in a separate syscall
merely narrows the window. A new local mutex is insufficient unless every
potential competing launcher participates, including standard-library spawns.
The choice of syscall or allocation strategy remains open pending reproduction
and platform verification; a helper process is not a predetermined requirement.

Linux is the native implementation environment. The current PTY function is
shared with macOS, so macOS behavior must be preserved and checked in CI. A
Linux-only fix must explicitly identify any remaining macOS gap rather than
claiming all Unix platforms are fixed. No parser, keymap, terminal rendering,
persistent-session protocol, or Windows process-launch redesign is required by
this issue. Keep process-group identity and bounded cleanup guarantees intact.

The existing `openpty` boundary replaced a manual allocation sequence that
failed on macOS with `ENOTTY`; `context/issues/resolved/terminal_panes.md`
records the diagnosis. Preserve initial sizing on the slave and correct
controlling-terminal acquisition after `setsid`. Do not reintroduce that old
sequence when selecting a Linux allocation path. The related FFI portability
record is `context/issues/resolved/macos_openpty_build.md`.

Before implementation, read `AGENTS.md`, the integrated-terminal part of
`docs/user-guide.md`, `context/README.md`, and
`context/reference/terminal-compatibility-v1.md`. Relevant implementation and
coverage live in `src/terminal/pty.rs`, `src/terminal/pending/`,
`src/process_group.rs`, `tests/terminal.rs`, and `tests/persistent_host.rs`.
Consult `src/git/cli.rs` only as needed to compare subprocess pipe ownership;
the existence of a PTY gap is not evidence that Git's Linux pipes have it too.

## Validation and delivery

The third work package validates the fix and records the native evidence.
Each work package receives independent subagent review, and actionable review
findings are incorporated before the next package begins.

Run the focused overlap regression and affected PTY/lifecycle tests on Linux,
then the repository handoff checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Preserve the canonical `cargo llvm-cov --locked --workspace` line-coverage
floor in `context/reference/test-coverage.md` on every affected first-class
target. Use remote CI for macOS validation and retain exact failing assertions
and test names if a platform remains unresolved. A passing stress run or
cross-compilation alone does not establish the inheritance guarantee.

Implementation proceeds on `dev` independently of Windows Git work on
`feat/windows-support`. Windows has a separate potential mixed-launch handle
inheritance problem; neither platform's failure is proof of the other's cause.
Keep the Unix implementation and regression commits focused so they can be
cherry-picked into `feat/windows-support`, and provide the exact hashes and
validation results at handoff. Do not merge unfinished Windows work into `dev`.
After the fix is committed, resolve this issue in the separate follow-up commit
required by `AGENTS.md`, preserving the diagnosis and any platform limitation.
