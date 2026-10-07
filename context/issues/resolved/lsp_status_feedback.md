---
title: "LSP status report is only visible in the notification buffer"
status: resolved
reported: 2026-10-06
resolved: 2026-10-06
commit: f66cf7b
---

## Resolution

Commit `f66cf7b` (`Show the LSP status report on the interaction line`)
routes the reply to `Space l ?` and `:lsp-status` back to the echo of the
command that asked for it.

The manager answered `LspCommand::Status` with the same `LspEvent::Status`
it uses for unsolicited messages: reconfiguration and restart notes,
`window/showMessage` warnings, and malformed-message errors. The editor
handled every one of those with `info_from` or `error_from`, so it could not
tell the report it had asked for from server chatter, and the report only
became an `INFO` notification while the interaction line kept
`Space l ? (Report language server state)`.

`LspCommand::Status` now carries `action: Option<u64>`, the dispatching
`active_action_id`, as an opaque value the manager echoes back in a separate
`LspEvent::StatusReport { action, message }`. The `Colon::LspStatus` arm in
`src/app/input.rs` fills it in. The new `StatusReport` arm in
`src/app/language_workflows.rs` calls the existing `update_action_feedback`,
the same helper Git and plugin results use, so the report replaces the echo
only while that action's echo is still the current one. Later keyboard or
text input, a pending prefix, or opening a prompt clears or replaces the echo
first, and a late reply then leaves the interaction line alone. The renderer's
existing interaction-line clipping cuts a long or multiline report with `...`.
`info_from("LSP", "Language server status", …)` still runs in every case, so
`:not` keeps the complete report, and it is the only record once the echo
has been superseded. Unsolicited `LspEvent::Status` messages are unchanged.

The id travels with the command rather than being queued on the editor side
because events from a revoked permission epoch are discarded on receipt, and
a FIFO of pending requests would then pair later replies with the wrong
action.

Tests in `src/app/tests/language.rs`:

- `an_lsp_status_report_replaces_its_current_echo_and_is_retained`
- `a_typed_lsp_status_command_reports_on_its_own_echo`
- `an_lsp_status_report_never_replaces_a_superseded_echo`
- `an_lsp_status_report_never_replaces_an_open_prompt`
- `an_unsolicited_lsp_status_leaves_the_echo_alone`

In `tests/lsp_client.rs`, the existing status tests now wait for
`LspEvent::StatusReport`, and two of them check that the manager returns the
`action` it was sent.

## Report

`Space l ?` and `:lsp-status` ask the language-server task for its state with
`LspCommand::Status` (the `Colon::LspStatus` arm in `src/app/input.rs`). The
task answers asynchronously, and its message lists every started or failed
server joined by ` │ `, for example:

```text
markdown: marksman (ready) │ rust: rust-analyzer (starting)
```

or `no language servers running` when there are none (`LspCommand::Status`
in `src/lsp/mod.rs`).

The editor handled that event with `info_from("LSP", "Language server status",
message)` in `src/app/language_workflows.rs`, which only pushes an `INFO`
notification. The interaction line kept the dispatch echo,
`Space l ? (Report language server state)`, and never showed the result. The
report could be seen only by opening `:not`, so the command looked like it did
nothing.

The one case that already reached the interaction line was the synchronous
refusal `language servers are not running`, when no language-server task
exists.

Expected: the report is visible right after the command runs, without opening
`[notifications]`.

The user guide already describes the same pattern for asynchronous Git
output: successful output updates the action echo while that echo is still
current, and multiline output, or output whose echo has been superseded, is
kept as `INFO`. Applied here:

- When the reply arrives and the `lsp-status` echo is still the current
  interaction-line content, replace its result with the report, e.g.
  `Space l ? (rust: rust-analyzer (ready))`.
- When the report is longer than the space left it is cut with `...` as other
  echoes are; `:not` still keeps the complete text.
- When the echo has been superseded by later input or a prompt, do not replace
  what is there now. The notification is then the only record.

Constraints:

- The reply is asynchronous, so whether the echo is still current has to be
  decided when the event is drained, not when the command is dispatched.
- An active prompt must never be replaced (`ui-vocabulary.md`: feedback never
  replaces the editable prompt text).
- `LspEvent::Status` is also used with `error: true` for server errors. Those
  must keep their current failure presentation; only the reply to an explicit
  status request should take over the echo.
- The `INFO` notification is always kept, including when the report was also
  shown on the interaction line. It preserves the full text for reports that
  had to be cut.
- No popup or list view. The interaction line for immediate feedback and
  `:not` for the complete report are sufficient, including when several
  servers are running.

Reproduction:

1. Open a Rust file in a project with `rust-analyzer` on `PATH`.
2. Press `Space l ?`.
3. The interaction line shows `Space l ? (Report language server state)` and
   nothing else changes.
4. `:not` shows an `INFO` entry from `LSP` titled `Language server status`
   with the report.
