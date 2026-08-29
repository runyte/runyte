# Test-scoped persistent hosts can outlive their test runner and escape session inventory

## Observed behavior

Integration tests start real `runyte --serve` children with private runtime and
cache roots such as:

```text
XDG_RUNTIME_DIR=/tmp/ryt-<test-runner-pid>-<nonce>
XDG_CACHE_HOME=/tmp/ryt-<test-runner-pid>-<nonce>/cache
```

This isolation correctly prevents tests from publishing endpoints into the
person's real persistent-session registry. The child is normally owned by a
`ChildGuard` whose `Drop` implementation kills and reaps it. If the test runner
is terminated without unwinding, however, that guard never runs. Multiple
`--serve` processes from completed test runs have remained alive for hours in
their private namespaces, each continuing to consume a small amount of CPU.

An ordinary `runyte --session-list` does not show these processes because it
scans only the runtime and cache registries selected by the current process's
environment. `runyte --session-stop-all` uses the same inventory and therefore
does not stop them. The hosts are live orphan processes rather than Unix
zombies, but there is no ordinary Runyte command in the production namespace
that can identify or retire them.

## Expected behavior

Test-scoped hosts should not survive the test runner that owns them. Cleanup
must cover abrupt runner termination as well as normal return and panic
unwinding. The ownership mechanism may be a parent-death signal where the
platform supports one, an external supervisor, a bounded lease or heartbeat,
or another design that does not weaken the deliberate isolation between tests
and real sessions. The behavior of manually supervised production `--serve`
processes must be considered separately rather than changed accidentally to
fit the test harness.

Runyte should also provide an explicit way to inventory all persistent-session
hosts owned by the current user, including hosts published in test-scoped or
otherwise non-default registry namespaces. A proposed spelling is:

```text
runyte --session-list-all
```

There must be a corresponding way to stop the additional hosts. Adding only
`--session-list-all` would leave the command family inconsistent because the
existing `--session-stop-all` currently means "stop every host visible in the
current registry namespace", not every Runyte host belonging to the user.

The persistent-session CLI should therefore be redesigned as one scope model
before either command is added. It remains undecided whether to:

- keep the current namespace as the default and add a common explicit
  all-namespaces option to list and stop operations;
- introduce paired list/stop commands whose names state the same scope; or
- redefine the existing `all` operation with a compatibility path.

Whichever syntax is chosen, help and the user guide must state whether "all"
means all hosts in the current namespace, all known production registries, or
all validated Runyte hosts owned by the current user.

## Reproduction

In an isolated test environment:

1. start an integration test that has spawned a real `runyte --serve` child;
2. terminate the test runner without unwinding;
3. verify that the child remains alive and that its socket and endpoint remain
   below `/tmp/ryt-<test-runner-pid>-<nonce>`;
4. run `runyte --session-list` and `runyte --session-stop-all` in the normal
   environment; and
5. verify that neither command reports nor stops the child.

## Constraints

- Keep test registries isolated from normal session selection, names, recent
  history, and lifecycle operations unless an explicit broader scope is
  requested.
- A comprehensive scan must be restricted to the current user and validate
  ownership, permissions, endpoint metadata, process identity, and symlink
  boundaries. A filename matching `/tmp/ryt-*` is not sufficient proof that a
  process is a Runyte test host.
- Listing must not mutate or remove a live endpoint merely because a sandbox or
  PID namespace cannot observe its process.
- Stopping must retain the existing protected-state and incompatible-protocol
  safeguards, with explicit force semantics for destructive cleanup.
- Normal test completion, panic unwinding, abrupt termination, and stale
  private runtime-directory cleanup all need regression coverage.
- Truly idle persistent hosts, including test hosts, should remain close to
  zero CPU between events.
