---
title: "No-host `--wait` attachment test can fail intermittently under lifecycle stress"
status: resolved
reported: 2026-08-31
resolved: 2026-08-31
commit: 4c13dd7
---

## Resolution

Commit `4c13dd7` (`Share one deadline across the suite's attachment waits`)
replaced the ad-hoc budget the failing poll ran on with the deadline the rest
of the suite already uses for asynchronous state.

The diagnosis is that the poll measured the wrong kind of wait. It ran 100
iterations of a 25 ms sleep, a shape that suits a host round trip. What it
was actually waiting for is a second process completing a multi-step
sequence: the test starts counting when the endpoint becomes connectable,
and from there the `--wait` client still has to finish its own control
handshake, send `CreateWait`, and connect a separate interactive client
before the host's `active` slot is filled and `interactive_attached` turns
true. Nothing in that sequence is racy — the host serialises it in one event
loop, and `handle_workspace_request` reports `active.is_some()` — but it is
long enough that a loaded runner can outrun a budget of that size. The suite
had a constant for exactly this class, `ASYNC_STATE_TIMEOUT`, and this poll
was not using it.

`wait_for_interactive_attachment` is the shape the fix settles on. It polls
health against the shared deadline; it ends immediately when the client
exits, because the attachment it was going to make can no longer arrive and
the exit status is the diagnosis that waiting out the deadline would bury;
and on either failure it reports the elapsed time, the last host health, the
client's live process state from `ps` and `/proc`, and the rendered terminal
plus a bounded tail of its raw output. It kills the client before failing, so
an editor still holding a PTY cannot outlive the test and perturb whatever
runs next in the same binary. It reads health through
`receive_semantic_response`, so an interactive connection carrying frames and
terminal damage can ask the same question a control connection asks. The
`preceded_by` argument names what the caller already waited through, which is
what separates a slow machine from a stalled attachment after the fact.

The fix deliberately covers more than the reported test, on evidence. Eight
concurrent copies of the suite at `--test-threads 8` under thirty busy loops
on a twenty-core machine reproduced the same mechanism in
`relative_workspace_attach_uses_editor_cwd_and_keeps_one_client_process`
(`the client did not return to the source`) and
`worktree_switch_reuses_the_destination_host_through_the_real_tui_launcher`
(`destination host was not reused`). Every asserted attachment poll in the
file was converted rather than only those two, so the boundary is statable:
an asserted wait for a terminal to become the interactive attachment now
shares the suite deadline everywhere in `tests/local_protocol.rs`.
`wait_for_requested_buffer` does the same for the four polls that waited on a
freshly spawned client's request reaching the host, and
`wait_terminal_hangup_is_not_reported_as_success` gained an assertion it had
been missing: it polled for the attachment and then made its hangup claim
whether or not one arrived.

One change goes beyond what the report asked for.
`wait_client_exits_when_its_launching_process_dies` broke its precondition
loop only when the launcher had published its child *and* the host held that
child's attached request, but on expiry asserted the published PID alone. A
run in which the attachment never arrived would proceed to make parent-loss
claims about a client that had never attached, reporting that defect for the
wrong reason. It now requires both, and its timeout names the published PID,
the last host health, and the launcher's process state. That is a
misattribution fix rather than a flakiness fix, and it is here because the
same loop had to be rewritten for its budget anyway.

Three supporting changes carry the diagnostics the report asked for.
`live_process_state` replaces three copies of the same `ps` and `/proc`
block. `TerminalCapture` records when its reading thread reaches the end of
the terminal, so `terminal_output_at_exit` can wait for that end before
deciding what a client said on its way out — the previous code read the
capture the instant it observed the exit and could miss the message the exit
was about, which mattered because the restricted-environment skip depends on
finding `Operation not permitted` there. `raw_tail` bounds captured output so
a failure message stays readable after seconds of full-screen redraws.

Verification: the named test passed 20 isolated runs, and the CI burn-in
shape — ten attempts over `local_protocol`, `diagnostic_log`,
`persistent_host` and `workspace_bulk` — passed 40 of 40 suite runs. Under
the saturating load described above, the final change passed 64 of 64
whole-suite executions, and the named test did not fail in any campaign after
the deadline was shared. `cargo fmt --check`, `cargo clippy --all-targets --
-D warnings` and `cargo test --locked` are clean. The behaviour is covered by
`wait_without_a_host_starts_one_and_attaches_the_invoking_terminal` in
`tests/local_protocol.rs`, and the shared helpers are exercised by every
attachment-waiting test in that file.

Known limitation: the correction identifies the test's budget as what made
the failure possible, not a lower-level cause in the editor, and no such
cause was found. A recurrence at the shared deadline is now diagnosable
rather than silent, which is what the report asked for, but it would still be
a new investigation. The load campaigns also surfaced three failures outside
this class, two of which are recorded as open issues:
`context/issues/interactive_handshake_first_response_is_not_the_welcome.md`
and `context/issues/persistent_tui_exits_before_reaching_a_new_worktree.md`.
Finally, the file keeps roughly eighteen ad-hoc bounded loops that settle
after the fact — the `released` and `cancelled` polls that run once a client
has already been confirmed dead, where the host has one scheduling hop left
rather than a multi-step sequence to complete. Those were left deliberately.
Two assertions of that kind have no budget at all, after the commit editor
exits and after a worktree switch reports its destination attached; they are
in the same untouched category.

## Report

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
