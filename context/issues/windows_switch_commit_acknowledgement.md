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
