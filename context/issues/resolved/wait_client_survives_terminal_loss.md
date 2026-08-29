---
title: "A --wait client can survive terminal loss and consume a CPU core"
status: resolved
reported: 2026-08-29
resolved: 2026-08-29
commit: cb6c083
---

## Resolution

Commit `cb6c083` (`End abandoned wait client lifecycles`) fixes the abandoned
client lifecycle. `run_attached` previously depended on Crossterm's
`EventStream` to report terminal loss, but Crossterm 0.29's Unix reader does
not turn a zero-byte terminal read into stream termination. A hung-up PTY
could therefore remain continuously readable without yielding an event,
leaving the client alive and consuming a core. The queued part of `run_wait`
also had no way to notice that the process which launched it had disappeared.

The wait client now starts a blocking exceptional-condition watcher for the
actual terminal source: duplicated standard input when it is a terminal, or
`/dev/tty` when standard input is redirected. The watcher reports only
hangup/error conditions, uses a cancellation descriptor for shutdown, and is
started after detached-host startup so it cannot interfere with the startup
fork. Wait mode also observes its launching process with stable kernel
facilities where available. Both queued and attached phases reconcile a
lifecycle loss with the durable wait status under a bounded deadline, so an
already-recorded completion wins the race while a genuinely abandoned request
is cancelled. The existing SIGHUP status retains priority when terminal loss
and signal delivery arrive together.

A later release-gate follow-up, `Fix wait client terminal cleanup`, closed two
races exposed by those regression tests. A pending wait is now owned by the
control connection that created it, so host-side disconnect cleanup cancels it
even when the client's final `CancelWait` frame and process exit overtake one
another. The wait attachment also reconciles a frame-write failure with the
terminal watcher and signal handler before choosing its exit status. When the
PTY is already unreachable, it suppresses Ratatui's destructor retry instead
of letting Ratatui report a failed cursor restore through the same dead stderr;
that report previously panicked and replaced the intended SIGHUP status with
exit code 101.

A later Darwin-specific refinement replaced the terminal descriptor's poll
registration with a native kqueue watcher. Requesting `POLLHUP` made an idle
PTY close observable on macOS, but Darwin implements poll through a one-shot
read knote; ordinary unread input could consume that observation before the
later close. The watcher now registers `EVFILT_READ` with `EV_CLEAR`, ignores
ordinary read events without reading the terminal, and reports loss only when
the event carries `EV_EOF`. The cancellation descriptor has its own read
filter and retains priority when cancellation and EOF arrive together. Linux
and other Unix targets continue to poll only exceptional descriptor state.
`src/main.rs::terminal_loss_watcher_observes_pty_peer_close` covers the direct
PTY-close boundary on both CI platforms, while the local-protocol subprocess
tests cover real wait clients with active terminal traffic.

A later release-gate follow-up isolates wait-mode input from the lifecycle
executor. The independent descriptor watcher was correct, but Crossterm's
`EventStream::poll_next` could synchronously block the single-thread Tokio
runtime while waiting for Crossterm's process-global reader mutex. Its reader
held that mutex forever when a dead PTY repeatedly returned EOF, so Runyte
could not receive the watcher's already-ready notification. A dedicated,
unjoined OS thread now performs blocking wait-mode input and forwards parsed
events over a channel. The thread is allowed to remain inside the third-party
reader until the dedicated wait process exits; terminal loss, host status,
signals, and request cancellation remain independently responsive.

Regression coverage lives in `tests/local_protocol.rs`:

- `attached_wait_client_exits_and_cancels_when_its_terminal_is_lost`
- `queued_wait_client_exits_and_cancels_when_its_terminal_is_lost`
- `handed_off_wait_client_exits_and_cancels_when_its_terminal_is_lost`
- `redirected_stdin_uses_dev_tty_for_terminal_loss`
- `controlling_terminal_loss_preserves_the_hangup_exit_status`
- `durable_completion_wins_a_race_with_terminal_loss`
- `terminal_loss_recovery_is_bounded_when_the_host_stops_responding`
- `wait_client_exits_when_its_launching_process_dies`
- `durable_completion_wins_a_race_with_launcher_loss`
- `wait_terminal_hangup_is_not_reported_as_success`

Known limitation: when the platform cannot provide stable kernel observation
of the launching process, Runyte falls back to checking its recorded process
identifier every 500 ms. As with other Unix parent tracking, loss that happens
before Runyte records the launcher cannot be distinguished afterward.

## Report

A `runyte --wait note.txt` process remained alive after the terminal and
development worktree that launched it were no longer present. The process was
runnable at approximately 100% CPU, meaning one complete logical core, and had
accumulated more than a day of CPU time. No editor request was still being
actively used.

This was not an ordinary Unix zombie: the process continued to execute and
consume CPU. It was an abandoned `--wait` client whose lifecycle no longer had
a person or terminal capable of completing the request.

The normal `--wait` loops request status at 100 ms intervals and should be
nearly idle. The observed full-core load was consistent with terminal input
repeatedly becoming ready after hangup without producing an event. Runyte uses
Crossterm 0.29 `EventStream`; its Unix reader does not turn a zero-byte
terminal read into stream termination.

A `--wait` client should exist only while it owns a reachable pending wait
request or a terminal attachment that can complete it. Losing the controlling
terminal must cancel or release the wait request according to the existing
failure semantics, restore any terminal state that remains reachable, and exit
nonzero within a bounded interval. It must not spin after EOF, hangup, host
failure, or loss of its launching process.

Normal pending waits must remain inexpensive. Status polling, terminal input,
and transport reads should block between real events rather than continuously
waking a runtime or helper thread.

A controlled reproduction should:

1. start `runyte --wait note.txt` in a disposable PTY with a real test-scoped
   persistent host;
2. leave the wait request pending and abruptly close the PTY master, without
   sending Runyte a normal detach or quit command;
3. assert that the client exits within a short deadline and that the host no
   longer retains its wait request; and
4. retain enough process diagnostics to distinguish a blocked client from a
   thread repeatedly polling and reading the closed terminal.

The failure could depend on whether the host already had an interactive TUI
and whether the waiting invocation took over after that TUI detached. Both
paths required coverage.

The fix had to preserve the documented handoff in which a pending `--wait`
invocation takes over the terminal after another interactive TUI detaches, as
well as explicit completion, cancellation, host-failure, and signal exit
statuses. Temporary absence of input could not count as terminal loss, and the
solution could not add another frequent polling loop. Subprocess coverage was
required so a stuck Crossterm reader could not hang the test runner itself.
