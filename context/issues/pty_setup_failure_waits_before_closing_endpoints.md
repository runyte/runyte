Unix PTY setup owns a spawned child with `SpawnedChild`, but keeps the master
and initially the slave descriptors in earlier local variables. If descriptor
duplication or a worker-thread spawn fails, the guard waits for the child before
those endpoints close. A writer already started for an unpublished terminal is
also waiting for activation. Its cancellation currently happens only when the
outer `TerminalPreparation` unwinds, after the child wait has returned.

On Darwin a session leader can wait for unread terminal output to drain during
exit, including after `SIGKILL`. The failure path can therefore deadlock: reaping
waits for endpoint closure while endpoint closure and gate cancellation wait for
reaping. The completed unpublished-terminal teardown already releases its master
before waiting, but the partial setup guard does not follow that ordering.
Ordinary post-spawn setup failures before a reader exists retain the same master
while waiting.

Own the initial endpoints in the setup guard, cancel unpublished worker gates
on setup failure, and close the guard's endpoints before reaping. Keep successful
publication unchanged and retain the pending accounting lease until child
cleanup and any gated duplicate-descriptor closures have completed.

Exercise failures at child ownership, reader duplication, writer duplication,
and writer startup. Verify cancellation, release of worker/accounting ownership,
and child reaping with an output-producing fixture. Native Darwin validation is
required for its terminal-drain behavior; Linux can validate the ownership and
cancellation contract.
