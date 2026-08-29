---
title: "PTY setup failures could leave spawned children running"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: e796d69
---

## Resolution

Commit e796d69 (`Reap PTY children after setup failure`) introduced an armed
child owner immediately after `Command::spawn`. Any later descriptor
duplication or thread-creation failure now closes owned descriptors,
terminates the child's process group with the normal HUP/KILL escalation, and
waits for the leader. Successful `Pty` construction disarms the owner only
after the reader thread exists and no fallible setup remains.

The cleanup path shares `terminate_child` with normal PTY termination so setup
failure, explicit shutdown, and `Drop` retain the same process-group behavior.

A later macOS lifecycle burn-in exposed an ownership gap after normal child
exit. `Pty::finished` reaped the leader, after which dropping that `Pty` still
sent `SIGHUP` and `SIGKILL` to its negative numeric PID. Once reaped, that
number no longer identified the terminal's process group and could be reused
by an unrelated child. `terminate_child` now checks the owned `Child`
immediately before signalling: completed children make teardown signal-free,
while a live unreaped leader continues to anchor process-group termination and
reaping.

Coverage lives in `src/terminal/pty.rs` in
`every_post_spawn_setup_failure_terminates_and_reaps_the_child`. Its injected
checkpoints cover failures before reader duplication, writer duplication,
writer-thread creation, and reader-thread creation. The surrounding PTY tests
in that module and terminal-session integration in `tests/terminal.rs` cover
the successful lifecycle.
`completed_child_teardown_never_signals_a_reusable_process_group` covers the
reaped-child boundary, and
`running_child_teardown_still_signals_and_reaps_its_private_group` retains the
live-child cleanup contract.

## Report

PTY integration and terminal-session ownership from spawn through exit
required a focused hardening review. The scope included
`src/terminal/pty.rs`, `src/terminal/mod.rs`,
`src/app/terminal_workflows.rs`, persistent-host integration, and terminal
tests.

The review covered argument and working-directory handling, descriptor
ownership, nonblocking I/O, input and output backpressure, resize races, child
exit and reaping, process-group behavior, shutdown escalation, detach and
reattach, session identity, pane replacement, orphan prevention, terminal
review mode, large paste operations, and cleanup on every error path.
Regression coverage was to distinguish deterministic session-state behavior
from platform-dependent PTY behavior.
