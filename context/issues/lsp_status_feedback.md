# LSP status report is only visible in the notification buffer

`Space l ?` and `:lsp-status` ask the language-server task for its state with
`LspCommand::Status` (the `Colon::LspStatus` arm in `src/app/input.rs`). The
task answers asynchronously with `LspEvent::Status`, whose message lists every
started or failed server joined by ` │ `, for example:

```text
markdown: marksman (ready) │ rust: rust-analyzer (starting)
```

or `no language servers running` when there are none (`LspCommand::Status`
in `src/lsp/mod.rs`).

The editor handles that event with `info_from("LSP", "Language server status",
message)` in `src/app/language_workflows.rs`, which only pushes an `INFO`
notification. The interaction line keeps the dispatch echo,
`Space l ? (Report language server state)`, and never shows the result. To see
it the report has to be opened with `:not`, so the command looks like it did
nothing.

The one case that already reaches the interaction line is the synchronous
refusal `language servers are not running`, when no language-server task
exists.

Expected: the report is visible right after the command runs, without opening
`[notifications]`.

The interaction line is the natural place for it. The user guide already
describes the same pattern for asynchronous Git output: successful output
updates the action echo while that echo is still current, and multiline output
or output whose echo has been superseded is kept as `INFO`. Applied here:

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
  `status_revision` already advances on every interaction-line change and is a
  candidate for that check.
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
