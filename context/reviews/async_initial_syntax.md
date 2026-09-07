# Asynchronous initial syntax review

Reviewed: 2026-09-07. Implementation based on `aee17b0`, reviewed as an
uncommitted working-tree change. The approved design is
`context/plans/completed/PLAN_ASYNC_INITIAL_SYNTAX.md`.

## Independent review, pass 1

Result: clean; no actionable correctness or performance findings.

The reviewer checked production startup and later opens, generation/revision/
language validation, edits during initial parsing, checkpoint reuse, bounded
request and completion queues, cancellation, non-blocking shutdown, syntax
command availability, live hint refresh, persistent host integration, and the
benchmark probe's parser-completion observation site.

The reviewer found the new deterministic tests cover the principal lifecycle
boundaries. It did not run concurrent Cargo builds or tests; validation and
performance measurements are recorded by the implementing agent separately.

## Validation follow-up

The first full-suite attempt inside the process sandbox failed a Git readiness
assertion and stalled in a local-socket handshake test. The suite was rerun
outside the sandbox, where those tests passed. Two new/affected presentation
expectations were corrected: a snapshot's document row may span multiple text
runs, and semantic outline invocation now shares the syntax capability's
unavailability message. Targeted tests and the subsequent complete ordinary
suite passed.

The complete ordinary and canonical coverage suites each passed 3,027 tests,
with 32 ignored. Linux total line coverage is 91.70%, above the unchanged 89%
floor. Formatting and Clippy passed. The startup-timing feature tests, all 12
benchmark-harness tests (including the built editor), and the selected release
performance tests passed. Native macOS validation is not available on this host.
The first performance comparison measured large Lua editing readiness at
23.1 ms after the change, versus 160.9 ms before and 31.7 ms for Neovim.
A subsequent implementation correction requires final remeasurement.

## Independent review, pass 2

Finding: P2 — an unread initial completion could own the last reference to a
large tree. Removing it in `SyntaxHandle::send`, `cancel`, or `Shared::stop`
could traverse and free that tree on the input thread while holding the queue
mutex. Checkpoint pruning could similarly hold the mutex during worker-side
cleanup. Shared ownership did not protect an initial tree that had never been
applied to editor state. Settled quit timing did not isolate destructor cost.

Correction: unread superseded completions remain hidden in their bounded
per-buffer slot until the worker can reuse a same-generation tree as a
checkpoint or retire it. Cancellation transfers tree ownership to worker-side
retirement, and shutdown signals the worker without clearing parser values on
the caller. The worker extracts retired values and expired checkpoints before
releasing them outside the mutex. Same-generation request replacement shares
its base with the new request and does not accumulate retired clones.

New deterministic tests hold disposal on the worker and verify request delivery
continues while cleanup is blocked, assert the disposal thread and lock
boundary, and exercise checkpoint reuse from an unread completion. The rest of
the second review was clean, including the early-quit harness and the recorded
sample counts/medians.

## Independent review, pass 3

Result: clean. The P2 finding is resolved; no additional actionable findings.

The reviewer confirmed unread initial completions can be reused as checkpoints
or disposed on the worker, cancellation transfers ownership, shutdown only
signals, and checkpoint destruction releases the queue mutex first. Retirement
is bounded by existing parser-owned trees; repeated typing replaces requests
sharing a base and cannot accumulate retired clones. The new tests cover
thread ownership, unlocked cleanup, continued input, and unread-checkpoint
reuse.

## Final validation

After the pass-2 correction, the ordinary and canonical coverage suites each
passed 3,029 tests, with 32 ignored. Total Linux line coverage is 91.69%
(101,394 of 110,586), above the unchanged 89% floor. Formatting and Clippy with
warnings denied passed. All 13 benchmark-harness tests passed against the final
release binary. The release initial-readiness test and both existing background
reparse performance tests passed. The instrumented release build also compiled
the `startup-timing` feature. Native macOS validation remains unavailable on
this Linux host.

Final measurements are recorded in
`context/reference/startup-performance.md`, with individual samples retained
under `benchmarks/results/`. Compilation, tests, and coverage finished before
the final timing runs.
