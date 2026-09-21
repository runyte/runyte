# macOS PTY descriptors may be inherited during allocation

The macOS branch of `src/terminal/pty.rs::open_pair` uses native `libc::openpty`
to allocate the master and slave, then sets `FD_CLOEXEC` on each endpoint with
separate `fcntl` calls. An unrelated process executing between allocation and
those updates can retain an endpoint. Updating the parent's flags afterward
does not change the child's copy. A retained endpoint can extend PTY lifetime
or affect shutdown and EOF behavior.

The remaining platform gap is documented in
[`terminal-compatibility-v1.md`, Unix PTY descriptor ownership](../reference/terminal-compatibility-v1.md#unix-pty-descriptor-ownership).
The Linux fix in commit `5e30ffb` uses atomic close-on-exec allocation; it does
not change the macOS path. Apple's published
[`openpty` implementation](https://github.com/apple-oss-distributions/Libc/blob/main/util/pty.c)
opens both endpoints without `O_CLOEXEC`. Native macOS flags and an actual
concurrent exec overlap still need verification. The Linux reproduction is
not evidence of a macOS runtime failure.

An unrelated executed program must not retain either PTY endpoint. The intended
terminal child must still receive stdin, stdout, stderr, its controlling
terminal and process group, and the requested initial size. Close-on-exec does
not prevent temporary inheritance between fork and exec; a child that forks
and deliberately never executes is outside this guarantee.

A deterministic native regression should hold allocation before the flag
updates, execute an unrelated compiled fixture through `std::process::Command`,
and identify retained endpoints by native device identity rather than descriptor
numbers alone. The fixture needs an explicitly owned control channel, execution
and observation acknowledgments, bounded waits, and cleanup on every failure
path. Linux's `/proc/self/fd` and `TIOCGPTN` probe in
`src/terminal/tests/pty_descriptors.rs` needs a native macOS equivalent. Fixed
sleeps, probabilistic stress, and cross-compilation do not establish the
inheritance guarantee.

The allocation strategy remains undecided pending native verification. A
separate flag update merely narrows the race. A local mutex cannot serialize
unrelated standard-library spawns. A helper process is not predetermined.
Preserve the native setup ordering: both endpoints exist and initial sizing is
applied to the slave before the child calls `setsid` and acquires its controlling
terminal. The earlier manual sequence failed with `ENOTTY`, as recorded in
[`terminal_panes.md`](resolved/terminal_panes.md); the FFI pointer requirements
are recorded in [`macos_openpty_build.md`](resolved/macos_openpty_build.md).

Validation must cover deterministic overlapping exec, allocation-failure
cleanup, terminal input/output, initial size, resize, process-group teardown,
and pending-terminal cancellation on native macOS. Keep the existing Linux
regression and the canonical Linux/macOS coverage floor intact. Fixtures must
use compiled or checked-in executables, temporary storage and configuration,
and deliberate control-descriptor inheritance, without changing process-global
environment variables in concurrent tests.
