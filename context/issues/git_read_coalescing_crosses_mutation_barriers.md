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
