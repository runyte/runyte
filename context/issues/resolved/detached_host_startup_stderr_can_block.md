---
title: "Detached host stderr can block startup beyond its readiness deadline"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 05f8f2a
---

## Resolution

Commit `05f8f2a` (`fix(session): drain detached host diagnostics with bounded ownership`).

ReapedChild retained an undrained stderr pipe during startup and read it to EOF after direct-child failure. A noisy child could block before readiness, while a descendant retaining the pipe could prevent failure reporting indefinitely. A nonblocking AsyncFd task now drains through the direct host lifetime, retains a 16 KiB diagnostic prefix, yields after bounded work, and performs at most a 64 KiB final drain on cancellation. The existing reaper owns cancellation after readiness and reserves its thread before launching a child. Native Windows startup uses NUL handles and does not share this pipe problem.

Coverage: startup_failure_does_not_wait_for_descendant_stderr_eof, startup_drains_large_diagnostics_with_bounded_retention, and detached_child_keeps_draining_diagnostics_after_readiness_owner_returns in src/workspace/lifecycle/startup_stderr_tests.rs use checked-in stand-ins with isolated storage and bounded cleanup. Both startup regressions failed before the fix. All six lifecycle tests pass with one owned helper ignored; racing_starts_for_one_workspace_both_reach_the_winning_host in tests/persistent_host also passes.

## Report

Detached Unix host startup pipes the child process's stderr but does not drain
it while waiting for the endpoint. A child that writes more than the pipe
capacity can block before publication and be reported as a startup timeout.
After the direct child exits, `ReapedChild::stderr_detail` performs an unbounded
blocking `read_to_string`; a descendant retaining the write descriptor can
prevent EOF and stall the caller indefinitely despite the readiness deadline.

Use a checked-in stand-in executable linked into temporary fixture storage.
One behavior writes a diagnostic, starts a sleeping descendant that retains
stderr, and exits unsuccessfully. Startup must promptly report the diagnostic
without waiting for the descendant to exit. Another behavior emits more than
the pipe capacity; startup must not wait on an undrained pipe, and retained
diagnostics must have a finite byte budget. Fixtures must isolate configuration
and runtime paths and clean up their process groups.

Drain diagnostics without blocking the editor or retaining unlimited output.
Keep race-safe readiness detection, useful startup errors, detached host
lifetimes, and child reaping intact. Apply equivalent bounds to native Windows
startup if its transport has the same ownership problem.
