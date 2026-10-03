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
