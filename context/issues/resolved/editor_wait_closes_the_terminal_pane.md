---
title: "Quitting an editor-wait document closed the pane whose terminal it covered"
status: resolved
reported: 2026-09-01
resolved: 2026-09-01
commit: 24fc082
---

## Resolution

Commit `24fc082` (`Return a pane to the terminal an outside request covered`)
separates a pane giving up its terminal because someone navigated from a pane
having it taken by a request that arrived from outside the editor.

There was no nested editor in the reported workflow. `runyte --wait` does not
draw its own TUI when the host already has one; the host opens the requested
file in the attached TUI's active pane, which is the terminal pane the
invoking program is running in. `App::host_open_files_with_refresh` activates
through `App::open_file`, which calls `Pane::retarget`, and `retarget`
unconditionally clears `Pane::terminal` — correct for a person asking for a
document, wrong for a request nobody in the editor made. The terminal session
stayed live with no pane showing it, so what looked like a child editor
running inside the terminal was the parent's own pane switching content.

`App::request_view_quit` then read that pane as an ordinary document view:
`active_terminal()` was already `None`, so it fell through to closing the
buffer and then the pane. The pane disappeared while its session kept
running, reachable only through the terminal list.

`Pane::covered_terminal` now records the document an outside request put over
a live terminal together with the terminal it covered.
`App::quit_to_covered_terminal` runs ahead of every other reading of `:quit`,
including the one that would stop a single-pane editor, and
`App::retire_buffer` uncovers on the same terms so `:close` and `:wbc` behave
alike. `Pane::retarget` clears the claim, so navigating the pane elsewhere
ends the detour and `:quit` closes the pane as it always did. Ordinary host
opens record a claim as well as `--wait` ones: a second client opening a file
is the same kind of outside request.

The claim names its document deliberately. A pane reaches another buffer
through `Pane::replace_closed_buffer` when its own is retired, which is not
the ask that ends a detour, so an unnamed claim would survive into a buffer
unrelated to the request and reveal the terminal under it. `uncover_terminal`
also drops a claim it cannot honour rather than leaving it standing, and
`App::split` does not let the copied pane inherit one, for the same reason it
does not inherit the terminal itself: one pty has one size, and two panes
holding the claim would race to reveal one session.

Tests, all in `src/app/tests/navigation_and_files.rs`:

- `quitting_a_wait_document_uncovers_the_terminal_it_replaced`
- `quitting_a_wait_document_over_the_only_pane_uncovers_rather_than_exits`
- `closing_a_wait_document_uncovers_the_terminal_it_replaced`
- `navigating_away_from_a_wait_document_ends_the_terminal_detour`
- `a_terminal_taken_by_another_pane_is_not_uncovered_again`
- `a_host_open_over_a_terminal_is_also_a_detour`
- `a_split_does_not_inherit_the_claim_on_a_covered_terminal`
- `a_spent_claim_does_not_reveal_the_terminal_under_a_later_buffer`

Known limitation: a terminal whose child has exited is still uncovered rather
than the pane being closed, showing that session's final screen in review.
An exited terminal remains a terminal session until explicitly closed, so this
is consistent with the rest of the terminal lifecycle, but it does mean a
pane can return to a dead session.

## Report

With a persistent session and `git config core.editor 'runyte --wait'`,
running `git merge` in an integrated terminal pane opened the merge message
for editing. Quitting that message with `:q`, or closing it with `:c`, closed
the pane containing the terminal.

The terminal session itself kept running. It could be found afterwards and
shown in another pane through `Space t t`, so the child was never signalled;
only the pane was lost.

Expected behavior: finishing with the merge message returns the pane to the
terminal it was showing, at the shell prompt where Git is completing the
merge. The pane stays.

The report described the workflow as one Runyte running a terminal running
another Runyte. Whether a second process draws anything was not established
in the report itself.
