---
title: "Test-scoped persistent hosts can outlive their test runner and escape session inventory"
status: resolved
reported: 2026-08-29
resolved: 2026-08-29
commit: d2ab1be
---

## Resolution

Commit `d2ab1be` (`Supervise test-scoped persistent hosts`) made host ownership
and inventory scope explicit. The host event loop previously had no ownership
signal: an internally detached `--serve` child was intentionally independent
of its immediate launcher, so a test runner terminated without unwinding could
leave that child serving indefinitely. `HostSupervisor::for_launch` now
distinguishes foreground hosts from Runyte's internally detached hosts. A
foreground host watches its parent, while integration endpoints pass a test
supervisor identity through detached startup. The host checks that identity on
the existing one-second idle tick and performs its normal graceful shutdown
and unpublication when the supervisor disappears; no additional timer or idle
wake-up was added. The internal `--detached-host` marker keeps ordinary
production persistent sessions independent of their short-lived launcher.

`LocalEndpoint::publish_metadata` also publishes each live host into an
owner-private inventory independent of XDG runtime and cache namespaces.
Ordinary discovery, names, recent history, and lifecycle commands continue to
use only the current namespace. The shared `--all-namespaces` option explicitly
widens `--session-list` and `--session-stop-all`; stopping still passes through
the existing protected-state, incompatible-protocol, and `--force` checks. The
inventory uses the same private-directory, non-symlink, bounded-metadata, and
endpoint-identity checks as the namespace registries. Registry scanning probes
the Unix socket before accepting a row and does not remove a responsive host
solely because its PID is hidden by another namespace. Graceful cleanup now
also removes an empty per-host endpoint directory.

The common scope option was chosen instead of the proposed
`--session-list-all` spelling so list and stop operations express the same
scope without changing the established default meaning of
`--session-stop-all`. `README.md` and `docs/user-guide.md` document that
default and the owner-wide opt-in.

Regression coverage is in
`tests/workspace_bulk.rs`:
`detached_host_exits_and_unpublishes_when_its_test_runner_is_killed`,
`child_guard_reaps_a_test_host_during_panic_unwinding`, and
`explicit_all_namespaces_lists_and_stops_hosts_outside_the_current_registry`.
`src/workspace/transport.rs` adds
`symlinked_owner_wide_inventory_is_refused_without_following_it`,
`an_unobservable_pid_does_not_remove_a_responsive_endpoint`, and inventory
publication and cleanup assertions in
`registry_lists_names_rejects_duplicates_and_preserves_a_name_across_restart`.
`tests/release_packaging.rs` verifies the public and hidden CLI surfaces.

Known limitation: owner-wide discovery covers live hosts. Stopped recent
history remains namespace-local because no live host exists to publish it.

## Report

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

The persistent-session CLI therefore needed one scope model before either
command was added. The report left these choices undecided:

- keep the current namespace as the default and add a common explicit
  all-namespaces option to list and stop operations;
- introduce paired list/stop commands whose names state the same scope; or
- redefine the existing `all` operation with a compatibility path.

Whichever syntax was chosen, help and the user guide needed to state whether
"all" means all hosts in the current namespace, all known production
registries, or all validated Runyte hosts owned by the current user.

The reproduction was:

1. Start an integration test that has spawned a real `runyte --serve` child.
2. Terminate the test runner without unwinding.
3. Verify that the child remains alive and that its socket and endpoint remain
   below `/tmp/ryt-<test-runner-pid>-<nonce>`.
4. Run `runyte --session-list` and `runyte --session-stop-all` in the normal
   environment.
5. Verify that neither command reports nor stops the child.

The constraints were:

- Keep test registries isolated from normal session selection, names, recent
  history, and lifecycle operations unless an explicit broader scope is
  requested.
- Restrict a comprehensive scan to the current user and validate ownership,
  permissions, endpoint metadata, process identity, and symlink boundaries. A
  filename matching `/tmp/ryt-*` is not sufficient proof that a process is a
  Runyte test host.
- Do not mutate or remove a live endpoint merely because a sandbox or PID
  namespace cannot observe its process.
- Retain the existing protected-state and incompatible-protocol safeguards,
  with explicit force semantics for destructive cleanup.
- Cover normal test completion, panic unwinding, abrupt termination, and stale
  private runtime-directory cleanup with regression tests.
- Keep truly idle persistent hosts, including test hosts, close to zero CPU
  between events.
