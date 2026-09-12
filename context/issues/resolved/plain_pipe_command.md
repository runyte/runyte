---
title: "Selected text cannot be piped through an ordinary shell command"
status: resolved
reported: 2026-09-08
resolved: 2026-09-12
commit: d031852
---

## Resolution

`d031852` (`Add asynchronous shell pipes for selections`) implements the
plain-text filter path. `parse_colon_command` previously had no pipe inventory
entry, and the reserved `ShellPipe` editor command was unavailable. Typed
`:pipe` and `:|` now capture the invoking buffer revision and operative selection
spans in `App::request_pipe`. Parsing preserves shell quotes and escaped trailing
spaces through both the native palette and semantic command routes.

`WorkspaceHost::sync_pipe` admits one job per workspace and runs process work off
the editor loop. Each selection gets a separate `/bin/sh -c` invocation in the
captured workspace root, with selected text on stdin. Nonblocking I/O drains
stdout and stderr while feeding input. Admission limits selections to 256,
command text to 16 KiB, combined input and stdout to 8 MiB each, and retained
stderr to 16 KiB per invocation. The whole job has a 30-second deadline. Invalid
UTF-8 stdout and any failed invocation reject every replacement. `:pipe-cancel`
cancels explicitly. Cleanup kills the owned process group before reaping its
leader and does not wait indefinitely for inherited output descriptors after
shell completion.

`WorkspaceHost::handle_pipe_completion` checks closure, revision, read-only state
and cancellation before calling `apply_expected_transaction` on the captured
buffer. Replacements form one undo step, preserve exact stdout, and do not
activate a pane or require a frame. A successful empty transaction is a no-op.
Host-owned admission and completion channels survive frontend detachment;
shutdown cancels and waits for the worker. The bare `|` binding remains reserved
and now points to the typed command. Editor-variable expansion and additional
Helix shell commands remain outside this implementation.

Regression coverage:

- `shell_filters_preserve_text_and_parse_only_the_command`,
  `failed_exit_invalid_utf8_and_output_limit_are_errors`,
  `sequential_inputs_share_one_output_budget_and_deadline`,
  `cancellation_and_success_clean_up_children_without_waiting_for_inherited_pipes`,
  and `large_stdin_and_stdout_are_drained_concurrently` in `src/pipe/tests.rs`.
- `both_spellings_preserve_newlines_and_undo_once_without_a_frame`,
  `unicode_reversed_selections_remain_independent_across_panes_and_attachments`,
  `stale_closed_readonly_and_cancelled_results_never_apply`,
  `failed_selection_is_atomic_and_empty_output_deletes`,
  `admission_and_shutdown_bound_the_workspace_job`,
  `command_quotes_trailing_escapes_and_captured_directory_reach_the_shell`, and
  `empty_buffer_empty_output_is_success_and_admission_limits_do_not_queue_work`
  in `src/workspace/host/tests/pipe.rs`.
- `pipe_completion_while_detached_and_reattachment_keep_one_invocation` in
  `tests/persistent_host.rs` verifies actual detached completion, one invocation,
  reattachment, single-step undo and unchanged on-disk text.
- `unsupported_key_and_direct_routes_share_the_semantic_unavailable_boundary`
  and `unsupported_key_binding_echoes_its_message_inline` in
  `src/app/tests/commands.rs`, and
  `every_complete_hint_description_fits_forty_four_terminal_cells` in
  `src/key_hints.rs`, cover updated feedback and command discovery.

Known limitation: External command side effects cannot be rolled back.
Processes that deliberately leave the owned process group are outside cleanup's
scope. Shell execution is Unix-only; native macOS validation remains a CI task.

## Report

Runyte does not currently provide a plain text filter command. The keymap
reference records `shell-pipe` as unsupported. Using an ordinary program such as
`sort` to transform selected text should not require plugin registration,
enablement, or an implementation of the experimental JSON plugin protocol.

The expected interface is `:pipe <shell-command>` with the alias
`:| <shell-command>`. Each selection's text should be sent to a separate invocation
of the command on stdin, and that selection should be replaced with its stdout.
Programs available through `PATH` should work without editor-specific wrappers.
Shell arguments, quoting, and pipelines should be supported; selected text must
travel through stdin rather than being interpolated into shell source.

Helix provides the same `:pipe` and `:|` spellings and documents piping each
selection to a shell command. Its command-line documentation describes passing
the command argument through to the shell without interpreting its quotes. These
are references for the interface, not a commitment to implement Helix's
editor-variable expansions or every related shell command:
[commands](https://docs.helix-editor.com/commands.html) and
[command-line parsing](https://docs.helix-editor.com/command-line.html).

For example, select the complete text `b\na\n` and invoke `:pipe sort`. The
selected text should become `a\nb\n`. Undo should restore the original text in
one step. Repeating the example with `:| sort` should behave identically. With
multiple selections, each should be processed independently and all replacements
applied as one transaction. Stdout should be preserved exactly, including
trailing newlines; successful empty output deletes the corresponding selection.

Execution must keep input and rendering responsive. The invoking buffer,
revision, and selections must be captured before starting work. Results must
apply through the existing transactional mutation path, without activating a
buffer or requiring a rendered frame. The entire result must be rejected if the
buffer changes, closes, or becomes read-only while the command runs. Switching
panes must never redirect output into another buffer. A failed invocation must
leave all selections' text unchanged and report a useful error, including
bounded stderr where available. External side effects of the command cannot be
rolled back.

The implementation must define shell selection, working directory, cancellation,
timeouts, output and concurrency limits, invalid UTF-8 handling, and process
cleanup. Work should belong to the workspace host in standalone and persistent
modes; attaching or detaching a TUI must not duplicate execution. Exact policies
for these details remain to be chosen during implementation. No additional
default key binding or other shell command is specified by this issue.

Acceptance coverage should include both command spellings, programs resolved
through `PATH`, quoted arguments and pipelines, multiple and reversed
selections, Unicode text, exact newline preservation, empty output, single-step
undo, nonzero exit and spawn failure, bounded output and cancellation, stale
results, buffer closure, pane changes, and persistent attachment lifecycle.
Process fixtures must use temporary storage and the checked-in stand-in
executable with behavior data, following `AGENTS.md`. Command discovery, the
user guide, and the keymap reference must describe the implemented capability.
