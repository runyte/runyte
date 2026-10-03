The benchmark harnesses inherit the caller's development trace settings despite
isolating editor configuration. `benchmarks/ptybench.py::_spawn` overlays its
environment onto the inherited process environment, and the plugin and session
benchmark environments also retain `RUNYTE_INPUT_TRACE`.

A debug editor started by the baseline PTY harness with `RUNYTE_INPUT_TRACE`
pointing to an existing fixture-owned file replaces its contents during startup.
In a reproduction, a 23-byte caller trace became a 1,802-byte benchmark trace.
The same inherited output path can affect plugin and persistent attachment
benchmarks. `RUNYTE_STARTUP_TIMING_FILE` and `RUNYTE_BENCH_EVENTS` similarly
redirect development measurements, and an inherited `RUNYTE_PARENT_CONTEXT`
can route a baseline child into an unrelated editor context.

Benchmark children should omit these inherited settings without changing the
parent's environment. Explicit instrumentation supplied by the benchmark, such
as a fixture-owned milestone event path, must still reach the child. Version
probes and persistent host cleanup commands should use the same clean base
environment.
