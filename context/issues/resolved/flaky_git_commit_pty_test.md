---
title: "git_commit_wait_tui_completes_through_write_quit fails intermittently under a full cargo test"
status: resolved
reported: 2026-08-15
resolved: 2026-08-18
legacy_commit: 5332c22
---

## Resolution

Commit `5332c22` (`Fix flaky git_commit_wait_tui_completes_through_write_quit`)
found two distinct problems in the test, only one of which is the fixed-sleep
gap the issue described.

The problem that actually produced the reported `exit status: 1` was that the
test never read from the PTY it drove Runyte through. Typing the commit
message and then `:wq` renders an insert-mode frame per keystroke, and once
`:wq` opens the command line, a 71-entry command palette on top of that; with
nothing draining the master side, that output can fill the PTY's buffer. When
it does, the attached editor's next frame write does not just block — it fails
outright with a broken pipe, and the process exits with an error instead of
completing the wait. Git sees the editor exit non-zero and reports "there was
a problem with the editor ... Please supply the message using either -m or -F
option", and exits 1 itself. This was reproduced directly while working on the
fix: repeated serial runs of the unmodified test intermittently hit exactly
the reported panic, and capturing the PTY output on failure — the diagnostic
the issue itself asked for — showed the editor printing `Error: Broken pipe
(os error 32)` right after `:wq` was typed, followed by git's editor-failure
message. The issue's own hypothesis of an empty commit message was reasonable
given the assertion fired before anything could report what happened, but the
reproduced failures point at the broken pipe instead. The fix adds the same
background drain thread the other PTY-driven tests in this file already use
for this reason, keeping the drained bytes in memory instead of discarding
them so a future failure can still be diagnosed from them.

The second problem is the one the issue described: the two fixed 75 ms sleeps
between keystroke phases had no relationship to how long the editor actually
took to catch up. The insert phase now polls `ClientRequest::ListBuffers` (to
find the `COMMIT_EDITMSG` buffer, since attachment alone does not mean the
buffer exists yet) and then `ClientRequest::ReadBuffer` on the `control`
connection already used by the attach loop, waiting until the buffer's text
actually contains what was typed. The escape phase could not be fixed the same
way: pressing Escape has no buffer-visible effect, so a `ReadBuffer` poll taken
immediately after writing it matches on the very first attempt and confirms
nothing — an earlier version of this fix did exactly that, provided close to
zero real delay, and reproduced the broken-pipe failure at a higher rate than
the original 75 ms sleep did. What needs guarding here is not host state but
the raw byte stream: if the escape and the following `:wq\r` land in the same
read on the editor's terminal input parser, a bare ESC immediately followed by
`:` is shaped like the start of an Alt/Meta-modified key and can be consumed
as one, leaving insert mode active so `:wq` is typed as text instead of
executed. There is no protocol signal for this, so the escape phase keeps a
duration-based wait (raised to 150 ms) and follows it with a `ReadBuffer`
sanity check that the message text was not corrupted, per the issue's own
fallback suggestion.

The final assertion, on a non-zero exit, now also captures the drained PTY
output and the commit buffer's last-known text in the panic message, so a
future intermittent failure from either mechanism is diagnosable from the test
output alone.

Verified with `cargo test --test local_protocol
git_commit_wait_tui_completes_through_write_quit` in
`tests/local_protocol.rs` (100 consecutive isolated runs, and 40 more with
four busy-loop processes running concurrently to approximate load), plus
three full `cargo test` runs with the target test passing in each. `cargo fmt
--check` and `cargo clippy --all-targets -- -D warnings` both pass.

A 2026-08-26 follow-up found that the remaining broken-pipe failure happened
after the commit buffer had been saved correctly and `:wq` had completed. The
host queues the terminal `WaitState` and closes the interactive attachment;
the attached client's periodic `WaitStatus` poll could race that close, fail
its write with `BrokenPipe`, and exit non-zero before reading the queued
completion. `attach_for_wait` now resolves an attachment error against the
same durable wait token over the independent control connection. A completed
request therefore remains successful, while a pending or cancelled request
still reports the attachment failure or cancellation. The existing
`git_commit_wait_tui_completes_through_write_quit` regression in
`tests/local_protocol.rs` passed 50 consecutive isolated runs after that
change, followed by three complete `cargo test` runs.

A 2026-08-28 persistent-quit follow-up fixed the corresponding server-side
delivery race. `finish_attached_quit` completed durable waits and queued the
interactive terminal response, but removed that connection from `active`
before `flush_connections` ran. A fast host shutdown could therefore exit
with `WaitState` or `ShuttingDown` still queued, leaving the attached client
to report `BrokenPipe` while its control recovery found that the host had
already stopped. Shutdown now retains the interactive sender and connection
identity until the common flush drains it. The regression
`an_interactive_quit_flushes_its_shutdown_response_without_a_control_client`
in `tests/local_protocol.rs` covers the direct interactive path, while
`git_commit_wait_tui_completes_through_write_quit` continues to cover the Git
wait workflow.

Known limitation: the escape-to-command-line transition still relies on a
fixed delay rather than an observed signal, because the protocol has no way
to report editor mode to a control client. Sustained scheduling delay past
150 ms between the escape and `:wq\r` writes could in principle still
reproduce the same misinterpretation; the sanity check after it will catch
such corruption and fail with a clear message rather than a confusing
downstream Git exit code, but it does not eliminate the underlying input race.
The later wait-status recovery applies only after the host has actually
completed the request; it cannot turn a mistyped, still-pending request into a
success.

## Report

`git_commit_wait_tui_completes_through_write_quit` in `tests/local_protocol.rs`
fails intermittently under a full `cargo test` and passes whenever it is run
on its own.

It failed during the gates for the 0.0.20 release on 2026-08-15:

```
---- git_commit_wait_tui_completes_through_write_quit stdout ----
thread 'git_commit_wait_tui_completes_through_write_quit' panicked at
tests/local_protocol.rs:724:5:
Git commit failed after :wq: exit status: 1
```

The same binary then passed the test in isolation, passed it twice more as a
whole file, and passed it again in a repeat full-suite run. Only the first
full-suite run failed, so the trigger appears to be load rather than anything
about the commit path itself.

The test drives Runyte as `GIT_EDITOR` through a PTY. It waits for the TUI to
attach by polling `ClientRequest::Health` until `interactive_attached` is true,
which is a real readiness check. From there it stops waiting on state and waits
on the clock instead: it writes `iPTY commit message`, sleeps 75 ms, writes
`\x1b`, sleeps 75 ms, then writes `:wq\r` (`tests/local_protocol.rs:714`
through `721`). Nothing confirms that the editor consumed each of those before
the next is sent.

When the suite saturates the machine, those fixed sleeps are the part that can
come up short. Attachment having been observed does not mean the buffer is
ready for input, and a `:wq` that overtakes the inserted text leaves Git with
an empty commit message, which is one way Git exits 1 here. The exit status is
what the test observed; the empty message is the likely path to it and has not
been confirmed, because the assertion fires before the run can report what Git
wrote.

The fix is to make the three keystroke phases wait on observable state the way
the attach loop already does, rather than on a duration — the host can be asked
what the commit buffer contains after the insert and after the escape, and the
test can proceed once it reads back what it typed. Failing that, the assertion
should capture Git's stderr and the buffer's contents so an intermittent
failure is diagnosable from the output alone instead of only reproducible under
load.

Relevant code: `tests/local_protocol.rs` in
`git_commit_wait_tui_completes_through_write_quit`, the `ClientRequest::Health`
attach loop and the keystroke sequence that follows it.
