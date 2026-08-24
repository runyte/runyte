---
title: "Terminal persistence tests time out waiting for the host under load"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: 8c749de
---

## Resolution

Commit 8c749de (`Stabilize persistent terminal host tests`) resolves the
intermittent failures. The host was not taking five seconds to answer a local
request. The shared `response` helper was also being used as the wait for an
unsolicited terminal frame, so its panic could not distinguish a stalled host
from a shell child that had not yet been scheduled to produce the state the
test expected.

Instrumented runs under CPU oversubscription kept welcome, frame,
resynchronization, detach, and control replies in the microsecond-to-low
millisecond range while reproducing the silent wait. The two tests had
different ways to enter that wait. `frame_containing` waited passively for the
external terminal program to emit output, even though a responsive host has
nothing new to send until that happens.
`hidden_terminal_output_while_detached_is_unread_after_reattach` also sent
`Ctrl-\`, `Space`, `t`, and `q` as separate requests. Scheduler contention can
span the key-hint state's deliberate 1.2-second prefix deadline, abandoning
`Space t q`; the terminal then remains visible and no later frame can satisfy
the test.

The new `frame_matching` helper polls terminal-owned state with an explicit
`Resynchronize` every 250 milliseconds. Each poll still has to receive a host
reply within the original five-second `HOST_RESPONSE_TIMEOUT`, while the
externally scheduled terminal process has a separate 30-second state deadline.
The hidden-output test now opens the pane's document through one semantic
request to hide the terminal, which creates the state the persistence test
needs without coupling it to key-prefix timing. This is not a Runyte/Helix
binding change; the documented `Space t q` behavior and its hint timeout are
unchanged.

Coverage is in
`tests/persistent_host.rs::terminal_pid_output_and_input_survive_detach_disconnect_and_reattach`
and
`tests/persistent_host.rs::hidden_terminal_output_while_detached_is_unread_after_reattach`.
Both passed twenty consecutive runs with eight CPU spinners, and the complete
`cargo test` suite passed after the change. Run `cargo test --test
persistent_host` for the integration boundary.

Known limitation: a machine that cannot schedule the terminal child within 30
seconds still fails the terminal-state assertion, but it no longer reports
that condition as a five-second host response timeout.

## Report

Two tests in `tests/persistent_host.rs` failed intermittently at the same
place: the `response` helper at `tests/persistent_host.rs:129` gave the host
five seconds to send anything back, and under a loaded machine the host
sometimes sent nothing in that window.

```text
thread 'hidden_terminal_output_while_detached_is_unread_after_reattach'
panicked at tests/persistent_host.rs:132:10:
host response timed out: Elapsed(())
```

`terminal_pid_output_and_input_survive_detach_disconnect_and_reattach` failed
the same way. Both tests drive a real `runyte --serve` host over the local
transport with a live terminal session attached, and both detach, wait, and
reattach.

Observed rates on a 20-core machine were:

- 15 runs of `cargo test --test persistent_host` alone, with no other load:
  no failures.
- 16 full `cargo test` runs, where twenty-odd test binaries run in parallel
  and each terminal test spawns its own host process: one failure.
- 8 runs of `cargo test --test persistent_host` with eight busy CPU spinners
  alongside: one failure.

The trigger was contention rather than a particular sequence of requests, and
the failure moved between the two tests. It was initially unknown whether five
seconds of silence meant the host was actually stalling or whether the test's
budget was too tight for a machine running that many hosts at once. Five
seconds is long for a local socket round trip, so measuring response time under
contention was necessary before changing the budget.

The behavior under test was terminal output surviving a detach and reattach.
No failure reported incorrect terminal contents; every observed failure was
the timeout.

The problem was not introduced by a recent change. Neither
`tests/persistent_host.rs` nor the host and terminal code it drives was touched
by the commits released in 0.0.37, and the same tests cover behavior that
shipped in 0.0.36.
