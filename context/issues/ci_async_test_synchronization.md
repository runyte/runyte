# CI tests rely on timing and transport output instead of semantic readiness

CI run 33257024146 failed on commit `1750d93` even though that commit changed
only `context/reference/startup-performance.md`. The Ubuntu and macOS failures
have separate causes, and the surrounding CI history contains additional
timing-sensitive integration tests that can fail without a product regression.

On Ubuntu,
`relative_workspace_attach_uses_editor_cwd_and_keeps_one_client_process` timed
out waiting for the raw PTY byte history to contain the contiguous text
`│ linked`. The editor had already switched to `linked/other.txt`, and the
rendered status later showed the expected branch. Ratatui emits incremental
screen patches, so cursor-addressing escape sequences can divide text that is
contiguous on the terminal screen. Searching the transport byte stream is
therefore not a valid observation of rendered state. The same raw-status wait
is present in the other persistent-session worktree tests, and the incompatible
host test also searches the raw stream for `E1`.

On macOS, all seven `diagnostic_log` failures stop while constructing the host
supervisor with:

```text
Error: cannot register host supervisor process queue
Caused by: Invalid argument (os error 22)
```

The macOS supervisor creates a kqueue containing an `EVFILT_PROC`/`NOTE_EXIT`
registration, then passes that kqueue descriptor to `AsyncFd::new`. Tokio's
default constructor requests readable and writable interests. A kqueue may be
nested in another kqueue only with `EVFILT_READ`, so the writable registration
is rejected with `EINVAL`. The supervisor must register the descriptor with an
explicit readable interest, and a macOS regression test must observe a real
child exit through that descriptor.

The Git commit PTY regression test retains a related known limitation: after
sending Escape, it sleeps for 150 ms before sending `:wq`. A scheduler delay
longer than the sleep can leave the editor in the wrong mode. The current
protocol frame already exposes the editor mode, so the test can wait for a
current semantic frame in Normal mode instead of using elapsed time.

The broader audit also exposed the same attribution error in
`detach_reattach_preserves_live_editor_and_refuses_a_second_tui`. CI run
33267109748 received a frame with one selection immediately after sending `*`
and asserted that it was the input's result. The protocol permits an older
replaceable frame to already be queued. The subsequent `Detached` response is
the causal FIFO barrier for the input, and the first frame after reattachment
is the behavior boundary that proves the two-selection state survived.

CI run 33267355386 exposed an unbounded production-side wait behind a semantic
Git-discovery barrier. After reaping the top-level Git process, the pipe
finalizer polled `kill(-pgid, 0)` until no process used that numeric process
group identifier. macOS could continue to report it as present during teardown
or after reuse, so the Git worker never delivered its completion event even as
the host continued serving frames. Process-group liveness by reusable integer
identifier is not a valid completion acknowledgement.

CI run 33267736646 exposed a test-sandbox race in `persistent_host`. All tests
in the integration binary shared one runtime directory, cache directory, and
all-host catalog. Host publication and session rename therefore contended on
the same global name lock even when the projects were unrelated. One test
timed out renaming its host while another could not publish its endpoint.
Distinct endpoint IDs do not isolate the shared catalog or its locks; every
test needs a private sandbox, while the processes within one test must share
that sandbox. Endpoint readiness must also include a completed control
`Welcome` handshake rather than stopping when the metadata file appears.

The macOS stress job in the same run exposed another response-attribution
error in the notification-history test. The test stopped at the first frame
with any unread error, then assumed the count belonged to the invalid `cd`
command it had typed. A background error could satisfy that predicate first,
and queued frames could obscure which request produced the observed state.
The test must acknowledge its own failing command semantically and identify
that command's notification before asserting detach and reattach behavior.

CI run 33272519457 reproduced that attribution error after Git failure
classification was added: the retained unread count was two rather than the
hard-coded one. Notification history is process-global state, so an exact
count does not identify an event. This test must run in a confirmed non-Git
workspace, use the failing command's semantic result and detach response as
FIFO barriers, and inspect the retained notification by its unique missing
path through `ListBuffers` and `ReadBuffer`.

CI run 33268205970 confirmed that per-test host catalogs removed the Ubuntu
lifecycle collision, then exposed another macOS Git-discovery failure during
the burn-in. The command palette remained at “this project is not in a Git
repository” for a freshly initialized repository. That text also represented
a failed discovery: the application discarded the typed error, marked
discovery complete, and made failure indistinguishable from authoritative
absence. In addition, a valid marker followed by successful but empty
`rev-parse` output was accepted as no repository. Successful child cleanup
also signalled a process group only after reaping its leader, when the numeric
group ID was no longer an ownership-safe identity. Git completion must retain
the exited leader until its group is finalized, required discovery fields must
reject empty output, and the capability snapshot must expose discovery failure
separately from marker absence.

CI run 33268796677 exposed a lock inversion between that lifecycle work and
Crossterm's terminal input stream. After a `--wait` client's PTY master was
closed, Crossterm 0.29 remained in its Unix read loop while holding the
process-global event-reader mutex. Polling `EventStream` on Runyte's
single-thread executor then blocked on that mutex, so the executor could not
receive the already-independent terminal-loss notification. The client stayed
runnable at one full core and retained its pending wait. Wait-mode input must
be isolated from the lifecycle executor until the upstream EOF and stream
handoff fixes are available in a release; a wedged third-party reader cannot
be allowed to block request cancellation.

The same run confirmed that the preceding Git completion change was not
portable as written. Multiple macOS tests continued receiving fresh host
frames for 30 seconds while the Git worker never returned from repository
discovery. Darwin's `waitid(WNOWAIT)` path was the new unbounded boundary.
Darwin must register an `EVFILT_PROC`/`NOTE_EXIT` observer immediately after
spawn, observe exit without reaping, stop the still-anchored process group, and
only then collect the child status. Pipe finalizer wakes must also take
priority over simultaneous pipe readiness: Darwin's poll adapter can otherwise
keep reporting stale readiness around EOF and prevent the reader join from
settling.

Full parallel validation exposed the corresponding stdin race. The gated
writer was released before the finalizer published child completion, and a
concurrently forked process could briefly inherit the close-on-exec pipe reader
long enough for the small test write to succeed. Kernel acceptance did not
prove that the completed Git child consumed the bytes. Finalization must be
published before gated workers are released, and a writer with remaining input
must reject that completed boundary even if a foreign inherited descriptor
makes the pipe writable.

CI run 33269779053 showed that registering the Darwin process knote after
`Command::spawn` still left a gap for short-lived Git commands. XNU's process
filter captures exit edges only after attachment. CI run 33270234080 then
showed both forms of that gap: a child that is already gone can make the
registration itself fail with `ESRCH`, while a successful registration can
race an exit that has set the public `PROC_FLAG_INEXIT` state but has not yet
become `SZOMB`. The observer must treat registration-time `ESRCH` as completion
for its exclusively owned, unreaped child, then snapshot `PROC_PIDTBSDINFO`
after successful registration and while polling. `PROC_FLAG_INEXIT`, `SZOMB`,
or snapshot-time `ESRCH` records irreversible completion without reaping the
leader; a live snapshot leaves every later exit covered by the installed
knote.

CI run 33270632231 showed the same worktree-test timeout after that observer
state machine was complete, but the test's `git_summary` predicate could not
attribute it to discovery. A summary is populated only by the refresh requested
after successful discovery; pending discovery, pending refresh, a typed Git
failure, and a lost worker completion all leave it absent. Tests that need a
Git-only command must wait for that command row's shared availability instead.
The timeout diagnostic must retain the row's reason, the long-running action,
the interaction line, and notification counts so a later failure identifies
the actual phase rather than being labelled as a process-observation failure.

CI run 33271126529 supplied that phase information on burn-in attempt 9. Git
discovery completed with ``git rev-parse --show-toplevel` failed` but no exit
code or stderr, which is the shape of a signal-terminated child rather than an
ordinary repository refusal. The Darwin observer treated `PROC_FLAG_INEXIT`
and `NOTE_EXIT` as completed states, even though XNU exposes both before
`SZOMB`, then sent `SIGKILL` to the process group including the still-exiting
leader. Darwin cleanup must level-query a zombie-aware process snapshot and
signal the group only after `SZOMB` makes the leader's wait status stable. The
command-row diagnostic must also recognize the failure suffix in a row whose
description precedes its availability reason.

CI run 33272088159 failed at the same discovery boundary after the stable
zombie observer had passed a complete macOS burn-in in run 33271776091. The
command-palette row is prepared for the current terminal width, however, so
its text can end at ``failed`` even when the underlying error continues with
an exit status and stderr. A clipped presentation value cannot classify a
process failure. Git failures must retain a Unix termination signal separately
from an exit code, redacted logs must preserve that classification, and this
test's failure path must read the full retained notification through the local
protocol before diagnosing the child lifecycle.

CI run 33272842849 then retained the full classification: the precondition
failed because `git rev-parse --show-toplevel` was terminated by signal 9.
Darwin's zombie snapshot includes both `pbi_status == SZOMB` and the raw
`pbi_xstatus`, but Runyte discarded that authoritative pre-cleanup status,
sent `SIGKILL` to remove remaining members of the owned process group, and
classified Git from the later reap status. The observer must preserve
`pbi_xstatus` only once the process is a zombie, retain it as the command's
status, and use the post-cleanup `Child::wait` solely to reap and diagnose any
status change. This keeps genuine external signals visible without letting
Runyte's descendant cleanup rewrite a successful Git result.

CI run 33273153073 passed the previously failing worktree-switch test after
that status-preservation change, then exposed the same ambiguous readiness
predicate in `persistent_tui_opens_async_log_and_shared_commit_detail`. Its
`git_summary` check waited not only for discovery but also for the unrelated
startup refresh, while pending discovery, pending refresh, a typed failure,
and repository absence all made the predicate false. Repeated resynchronization
also generated new frame identifiers without proving that asynchronous state
had advanced. The Git log test must wait for the `git-log` command row's shared
availability, which is the exact prerequisite for invoking that command and
becomes true as soon as repository discovery succeeds.

CI run 33269246467 also showed that the full-content-budget performance gate
could fail on a single 71.93 ms sample against its 64 ms release budget. The
picker's score comparator recomputed each candidate's Unicode character count
throughout an `O(n log n)` sort, even though candidate text is immutable for
the lifetime of an entry. Candidate character counts must be cached at
admission. The performance gate must execute a fixed odd sample set from the
same pre-keystroke state after one warmup, report every sample, and compare the
median with the budget. This distinguishes a sustained regression from an
unrelated scheduler interruption without retrying a failure into success.

The expected behavior is that asynchronous integration tests wait on semantic
state or explicit process acknowledgements with bounded deadlines. PTY output
must still be drained to prevent backpressure and retained for failure
diagnostics, but raw byte substrings must not be used as readiness signals.
Replaceable host frames must be resynchronized before they are attributed to a
request. Every spawned process must have bounded cleanup. A failed CI job must
remain a failure rather than being retried into green.

Reproduction:

1. Run CI for commit `1750d93` on Ubuntu and macOS, as in Actions run
   `33257024146`.
2. On Ubuntu, observe the timeout for `│ linked` even though the captured PTY
   stream contains the destination path and fragmented branch-status updates.
3. On macOS, observe `EINVAL` from `HostSupervisor::new` before each affected
   diagnostic-log test can start its host.
4. Repeating `cargo test --locked --test local_protocol` locally can pass every
   time because the Ubuntu failure depends on redraw and scheduling order; a
   local pass does not invalidate the transport-level race.

## Recommended execution order

1. Register the macOS process kqueue with `AsyncFd::with_interest` and
   `Interest::READABLE`, then add a focused macOS child-exit regression test.
2. Replace raw PTY status waits in all persistent-session worktree tests with
   current `HostFrame` predicates. Continue to exercise keyboard input through
   the PTY and inspect generated buffers through the control protocol.
3. Replace the Git commit test's 150 ms Escape delay with a wait for a current
   semantic frame whose editor mode is Normal.
4. Audit integration-test sleeps and raw-output substring assertions. Sleeps
   may drive intentionally time-dependent product behavior, but must not be
   used as readiness barriers or absence proofs when an acknowledgement can be
   observed.
5. Centralize deadline-based test helpers that resynchronize replaceable
   frames, drain PTYs, report the last semantic state on timeout, and bound
   child cleanup.
6. Repeat the affected process and PTY tests under load on Ubuntu and macOS,
   then run the full formatting, Clippy, and test gates. A single successful
   run is not sufficient evidence that a timing failure is gone.
7. Do not use retries, longer arbitrary sleeps, suite serialization, or test
   thread reduction as fixes. Require a green CI result for the exact commit
   before a release is tagged or published.
