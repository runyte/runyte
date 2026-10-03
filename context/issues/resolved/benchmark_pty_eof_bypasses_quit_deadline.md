---
title: "PTY EOF bypasses the benchmark quit deadline"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: f96d899
---

## Resolution

Commit `f96d899` (`fix(benchmarks): retain child exit deadlines after PTY EOF`).

measure_startup no longer switches from nonblocking child observation to waitpid with no deadline when terminal output closes. It stops selecting the permanently readable PTY after EOF, continues WNOHANG polling within the existing quit budget, and invokes owned-child cleanup when that budget expires. EOF alone cannot create a successful quit sample.

Coverage: test_terminal_eof_does_not_bypass_the_quit_deadline in benchmarks/test_ptybench.py runs a real Python child that ignores SIGHUP, closes its terminal descriptors and remains alive. It exceeded the regression deadline before the fix; all 17 PTY harness tests pass afterward.

## Report

The startup benchmark can wait indefinitely after the editor closes its PTY.
`benchmarks/ptybench.py::measure_startup` treats terminal EOF as proof that the
child is about to exit and calls blocking `waitpid(pid, 0)`. Closing the terminal
does not require the process to exit, so this call bypasses
`QUIT_TIMEOUT_SECONDS` and prevents the benchmark from cleaning up a stuck
child.

A child that ignores SIGHUP, closes descriptors 0, 1, and 2, then sleeps for one
second takes about one second to collect even when `QUIT_TIMEOUT_SECONDS` is
50 milliseconds. A child that never exits leaves the benchmark blocked.

The benchmark should continue polling the owned child's exit status after PTY
EOF, respect the quit deadline, and terminate and reap the child if the deadline
expires. Closing the PTY without a successful process exit must not produce a
valid quit sample.
