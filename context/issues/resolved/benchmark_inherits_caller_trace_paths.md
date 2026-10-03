---
title: "Benchmark children can overwrite caller trace files"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: fd151ef
---

## Resolution

Commit `fd151ef` (`fix(benchmarks): isolate inherited tracing and routing settings`).

PTY spawn previously overlaid benchmark settings onto the inherited environment; omission could not remove caller tracing or context routing. A shared clean_environment snapshot now removes inherited trace, startup timing, milestone and parent-context settings. Children replace their own environment with this clean base plus explicit benchmark instrumentation. Plugin/session environments and version probes use the same base, without modifying the parent process.

Tests: all EnvironmentTests in `benchmarks/test_environment.py` check a real PTY child, explicit instrumentation, unchanged parent settings and version/plugin/session environments. The native readiness acceptance in `benchmarks/test_startup.py` now verifies the caller trace remains byte-identical after successful plain-text and Lua editor launches. The failure reproduced as a 23-byte trace replaced by 1,802 bytes; after the fix its 23 bytes remain unchanged. The focused group passes 43 tests when native acceptance is enabled.

## Report

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
