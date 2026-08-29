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
