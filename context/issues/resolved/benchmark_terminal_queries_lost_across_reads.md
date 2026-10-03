---
title: "PTY benchmarks lose fragmented terminal capability queries"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: f9d414e
---

## Resolution

Commit `f9d414e` (`fix(benchmarks): retain terminal queries across PTY read boundaries`).

The baseline startup, idle and persistent navigation paths treated each PTY read as a complete escape stream. TerminalQueries now shares the readiness benchmark’s bounded 256-byte suffix logic and persists across reads and observation phases, answering each completed request once. The readiness terminal delegates to that same implementation.

Tests: TerminalQueryTests and `test_split_terminal_query_does_not_stall_document_startup` in `benchmarks/test_ptybench.py` cover every split of ten query forms, once-only answers, bounded recovery and a real child blocked on a split query. The PTY/startup group passes 33 tests with two existing acceptance skips. Independent differential review also checked 5,000 streams over 102,611 read boundaries.

## Report

The baseline startup, idle, and persistent attachment benchmarks answer terminal
capability queries independently for each PTY read. PTY reads do not preserve
escape-sequence boundaries. A query split across two reads is therefore ignored,
which can stall an editor waiting for its answer and invalidate startup or quit
measurements.

For example, a child writes `ESC [` and then `6n` after a short delay, waits for
the cursor-position response, and only then writes its document marker. The
baseline harness never sends `ESC [ 1 ; 1 R`, never observes the document, and
times out. The readiness benchmark already retains a bounded stream suffix to
handle this case, but the other harnesses do not.

All PTY benchmark paths should retain incomplete capability queries across reads
and phase boundaries, answer each completed query once, and keep retained data
bounded. Ordinary document output must not cause duplicate answers.
