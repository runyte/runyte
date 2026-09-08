# Pipe selections through an external command

Runyte does not currently provide a plain text filter command. The keymap
reference records `shell-pipe` as unsupported. Using an ordinary program such
as `sort` to transform selected text should not require plugin registration,
enablement, or an implementation of the experimental JSON plugin protocol.

Add `:pipe <shell-command>` with the alias `:| <shell-command>`. Send each
selection's text to a separate invocation of the command on stdin and replace
that selection with its stdout. Programs available through `PATH` should work
without editor-specific wrappers. Support shell arguments, quoting, and
pipelines; selected text must travel through stdin rather than being
interpolated into shell source.

Helix provides the same `:pipe` and `:|` spellings and documents piping each
selection to a shell command. Its command-line documentation describes passing
the command argument through to the shell without interpreting its quotes.
These are references for the interface, not a commitment to implement Helix's
editor-variable expansions or every related shell command:
[commands](https://docs.helix-editor.com/commands.html) and
[command-line parsing](https://docs.helix-editor.com/command-line.html).

For example, select the complete text `b\na\n` and invoke `:pipe sort`.
The selected text should become `a\nb\n`. Undo should restore the original
text in one step. Repeating the example with `:| sort` should behave identically.
With multiple selections, process each independently and apply all replacements
as one transaction. Preserve stdout exactly, including trailing newlines;
successful empty output deletes the corresponding selection.

Execution must keep input and rendering responsive. Capture the invoking
buffer, revision, and selections before starting work. Apply through the
existing transactional mutation path, without activating a buffer or requiring
a rendered frame. Reject the entire result if the buffer changes, closes, or
becomes read-only while the command runs. Switching panes must never redirect
output into another buffer. A failed invocation must leave all selections'
text unchanged and report a useful error, including bounded stderr where
available. External side effects of the command cannot be rolled back.

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
Use temporary storage and the checked-in stand-in executable with behavior
data for process fixtures, following `AGENTS.md`. Update command discovery,
the user guide, and the keymap reference when the capability is implemented.
