---
title: "Native Windows session switching loses a committed receipt"
status: resolved
reported: 2026-09-27
resolved: 2026-09-27
commit: 2847aa3
---

## Resolution

Commit `2847aa3` (`Fix native Windows switch receipt recovery`) fixes the
frontend's interpretation of a source connection closing during a switch.
`Clients::commit_switch` queues `NativeSwitchCommitted` and disconnects the
source attachment. The independent buffered transport can read that receipt
and EOF before its writer observes flush completion. It correctly reports a
lost local write acknowledgement while retaining the semantic receipt.

The frontend's former `commit_switch` called `send_or_exit`, whose recovery
recognized only ordinary attachment endings. It consumed and discarded the
switch receipt, then reported that the host had exited without ending the
attachment. The process-exit branch of `await_switch_receipt` had the same
problem. A closed attachment did not establish that its host process had exited.

The private exchange in `src/tui/windows_frontend/switch.rs` instead reads the
buffered replies under the operation deadline and final-reply budget. Only a
matching receipt for the requested commit or abort establishes completion;
missing, stale, unrelated and error responses still fail. Parent acceptance
still requires confirmation on the same connection. If that confirmation's
write acknowledgement is lost, a matching final commit receipt is required;
acceptance alone cannot establish the handoff. A successful confirmation keeps
the existing rule that a lost final source receipt cannot undo the handoff.
The transport does not treat EOF as a successful write or retry uncertain input.

Full native validation also exposed premature connection teardown in the
Windows service tests' `answer` helper. It dropped its response sender just
after queuing `Health`, allowing the mock server to close while the client was
still flushing its request. The helper now retains that sender until the
matching client disconnects, under its existing five-second timeout. This
matches a running host's connection lifetime. Worktree assertions now print
unexpected results instead of hiding the underlying error.

Formatting, warnings-as-errors all-target Clippy, and the full native Windows
test suite pass after merging `dev` at `3a75e29`. Astra-high review found no
blocking findings in either the receipt recovery or the fixture correction.
Linux/macOS production code and their coverage thresholds are unchanged.

Regression coverage:

- `switch_receipt_survives_eof_before_local_write_acknowledgement` in
  `src/workspace/windows_transport/buffered/tests.rs` deterministically holds
  flush completion until the host has received the complete request and sent
  its receipt plus EOF. The local send fails and the receipt remains readable.
- `matching_receipt_recovers_lost_write_ack_before_eof`,
  `lost_write_ack_does_not_turn_missing_stale_or_unrelated_replies_into_success`,
  `parent_acceptance_requires_confirmation_or_matching_final_receipt`, and
  `silent_source_recovery_keeps_the_operation_deadline` in
  `src/tui/windows_frontend/switch/tests.rs` cover the bounded exchange and
  uncertain outcomes.
- `health_fixture_retains_the_connection_until_the_client_consumes_its_reply`,
  `parent_attach_selector_retains_the_fresh_exact_live_publication`, and
  `worktree_inspection_finds_a_ready_only_host_and_refuses_unreviewed_stop` in
  `src/workspace/windows_service/tests.rs` cover mock-host lifetime and both
  affected service workflows.
- `native_frontend_switches_between_exact_running_hosts` in
  `src/tui/windows_frontend_acceptance.rs` covers real native hosts and a ConPTY
  frontend switching, editing, recovering from refusal, and handling lost
  commit receipts.

## Report

The Native Windows job in [CI run 36345650111](https://github.com/runyte/runyte/actions/runs/36345650111/job/108694258312)
failed on commit `55be557`. The other 19 jobs passed.

`windows_frontend_acceptance::native_frontend_switches_between_exact_running_hosts`
in `src/tui/windows_frontend_acceptance.rs` recovered from an occupied destination,
wrote `RECOVERED` in workspace A, and displayed `WORKSPACE_B`. The next
`insert_and_write("B_EDIT ")` waited for `INS`, but the frontend exited with
code 101. Its error chain was:

```text
native workspace send failed and no final attachment response arrived:
native workspace host exited without ending the attachment
native buffered transport write acknowledgement lost
channel closed
```

Switching to an available persistent session should retain its attachment and
permit editing after the source acknowledges the switch. A source may close its
attachment immediately after committing; its matching switch receipt must remain
usable even if the local write acknowledgement or process-exit notification races
with receipt delivery. Missing or mismatched receipts must not count as success,
and parent handoffs must retain their two-phase confirmation requirement.

The failing test can be run on native Windows with:

```sh
cargo test --locked --bin runyte windows_frontend_acceptance::native_frontend_switches_between_exact_running_hosts -- --exact --nocapture
```

The failure is intermittent: eleven local runs at `55be557` passed. The CI job
uses `RUST_TEST_THREADS=2`; the report does not establish a reliable frequency.

Full native validation after merging `dev` at `3a75e29` also exposed two
failures in `src/workspace/windows_service/tests.rs`, both using the `answer`
health fixture:

- `parent_attach_selector_retains_the_fresh_exact_live_publication` returned
  `The pipe is being closed. (os error 232)` while preparing its live target.
- `worktree_inspection_finds_a_ready_only_host_and_refuses_unreviewed_stop`
  failed its assertion that the preparation error contains
  `changed after confirmation`. The assertion did not print the actual result.

The health fixture should preserve the mock host's connection while the client
consumes its reply, as an ordinary running host does. Fixture teardown must not
introduce an unrelated pipe-close race into service behavior tests.
