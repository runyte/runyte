---
title: "Git reads coalesce across queued repository changes"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 272f12c
---

## Resolution

Commit `272f12c` (`fix(git): keep read coalescing behind queued mutations`)
limits equivalent-read sharing to the final uninterrupted read-only segment
of the repository queue. A read can join an active read only when no queued
mutation or reconciliation intervenes; queued reads are searched backwards
only as far as the nearest such barrier. Previously a new post-change request
could join a pre-change read and receive its snapshot before the queued change
ran. Work remains ordered per common repository, and equivalent reads within
one segment still coalesce.

`read_coalescing_preserves_queued_mutation_and_reconciliation_barriers` in
`src/git/service.rs` failed before the fix and passes after it. Its controlled
worker covers active and queued equivalent reads with both mutation and
reconciliation barriers, and checks independent execution and completion order.
All 18 Git service tests passed, including existing coalescing and cancellation
coverage. The parent reviewer checked admission, repository keys, active reads
and queued-read lifetime independently.

## Report

Equivalent Git reads can coalesce across a queued repository mutation or
post-filesystem-change reconciliation. The scheduler checks active reads and
all queued reads by key without considering an intervening barrier in that
repository's queue.

For example, hold an active repository refresh, submit a stage operation,
then submit the same refresh again. The final request joins the active
pre-stage read and receives its older snapshot before the stage operation
runs. The same error occurs when the first equivalent read is queued behind
another active operation. `Reconcile` requests are intended as freshness
barriers but are crossed in the same way.

Reads submitted after a mutation or reconciliation must execute after that
barrier. Coalescing is still desirable between equivalent reads in the same
uninterrupted read-only segment. Keep the existing per-common-repository
ordering, worker concurrency, cancellation ownership, and mutation duplicate
checks.

A deterministic controlled worker can reproduce both active-read and
queued-read cases: hold the repository's first worker, enqueue an equivalent
read, a stage or reconciliation barrier, and another equivalent read, then
release execution after the last request is admitted. Every read separated by
the barrier must run independently and complete in repository order.
