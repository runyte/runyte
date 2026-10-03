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
