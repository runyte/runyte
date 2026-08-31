# No-host `--wait` attachment test can fail intermittently under lifecycle stress

The Ubuntu lifecycle-stress job can intermittently fail in
`tests/local_protocol.rs::wait_without_a_host_starts_one_and_attaches_the_invoking_terminal`.
GitHub Actions run
<https://github.com/runyte/runyte/actions/runs/33365936850> reached the failure on
burn-in attempt 9 of 10 after the preceding eight attempts passed:

```text
thread 'wait_without_a_host_starts_one_and_attaches_the_invoking_terminal' panicked at tests/local_protocol.rs:2564:5:
the no-host wait request never attached its terminal
```

The test successfully connected its control client to the newly published
workspace host, then polled `ClientRequest::Health` 100 times with 25 ms sleeps.
None of those responses reported `interactive_attached: true`, so the nominal
2.5-second attachment window expired. The assertion does not report the wait
child's status or the PTY output already captured by the test. The run therefore
does not establish whether the child remained alive but was delayed before the
interactive handshake, became stuck in the attachment path, or exited after
publishing the host.

The run's triggering commit, `fb60612` (`docs: add roadmap and project
philosophy`), changed only `README.md`. Every ordinary Ubuntu gate passed, the
macOS lifecycle-stress job passed all ten attempts, and the immediately
preceding commit passed the same Ubuntu lifecycle-stress job. During the initial
investigation at `fb60612`, the exact failing test passed 30 consecutive local
runs and the complete `local_protocol` integration suite passed 10 consecutive
runs. This evidence makes the failure intermittent but does not identify its
lower-level cause.

The expected behavior is that the no-host `--wait` lifecycle and its test are
deterministic under the production deadlines and normal test parallelism used
by the lifecycle-stress job. The stress gate must not be weakened with retries,
ignored failures, or reduced parallelism. A correction must distinguish an
actual client/host attachment race from runner scheduling delay, retain the
assertion that the invoking terminal becomes the interactive attachment, and
emit enough child status, host state, and bounded PTY output on failure to make
any recurrence diagnosable.

Reproduce with the same burn-in command used by `.github/workflows/ci.yml` on
Ubuntu:

```sh
for attempt in {1..10}; do
  cargo test --locked --test local_protocol
  cargo test --locked --test diagnostic_log
  cargo test --locked --test persistent_host
  cargo test --locked --test workspace_bulk
done
```

An isolated run may not reproduce the failure:

```sh
cargo test --locked --test local_protocol \
  wait_without_a_host_starts_one_and_attaches_the_invoking_terminal -- --exact
```
