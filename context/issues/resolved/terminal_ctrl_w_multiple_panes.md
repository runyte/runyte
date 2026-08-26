---
title: "Ctrl-w pane navigation could show stale terminal review in multi-pane layouts"
status: resolved
reported: 2026-08-21
resolved: 2026-08-21
legacy_commit: 43e99af
---

## Resolution

Commit `43e99af` (`Keep terminal input live across pane focus`) fixed the
transition between application-wide Insert mode and terminal-owned review
state. `App::focus_from_terminal_insert` and
`App::next_window_from_terminal_insert` preserved Insert when navigation
landed on another terminal, but they did not discard a review snapshot that
the destination session had captured earlier. Building a pane layout after
leaving a terminal with `Ctrl-\` therefore made the problem appear tied to a
terminal's position: returning to that session with `Ctrl-w` produced the
contradictory `[insert] [review]` state.

`App::finish_insert_window_command` now settles both pieces of state after a
window command. Any Insert-mode command that lands on a terminal discards its
stale review, scrolls to the live screen, and keeps Insert so input reaches the
child. A command begun from Terminal Insert that lands on a document still
enters Normal, preserving the existing protection against editing the
destination accidentally. The same boundary covers directional focus,
next-window cycling, and split creation.

Coverage lives in `tests/terminal.rs` tests
`control_w_focus_preserves_review_until_an_insert_key`, which exercises
side-by-side and stacked panes through `Ctrl-w h`, `Ctrl-w k`, and `Ctrl-w w`
and verifies subsequent child input, and
`control_w_from_document_insert_preserves_terminal_review`,
which covers entering a reviewed terminal from an Insert-mode document.

A later audit found a second route to the same contradictory state:
`App::show_terminal` entered Insert after the terminal manager, resource finder,
or `:terminal-show` selected a session, but it did not discard review captured
before that session was hidden or moved. The next pane movement exposed the
stale snapshot and made the movement appear to have entered review. Terminal
activation now calls `TerminalSession::scroll_to_live` before entering Insert,
so every existing-session activation has the same live-input boundary as pane
focus. `tests/terminal.rs` test
`showing_a_reviewed_terminal_preserves_review` covers
the hide, manager reopen, and subsequent `Ctrl-w h` sequence.

A later terminal-mode correction made directional focus stop in live Normal
instead of preserving Insert on a terminal destination. `Ctrl-w h/j/k/l` and
the configured fast `Ctrl-h/j/k/l` now share that command boundary: both leave
terminal input and focus the destination immediately, while neither captures
review. At that stage, existing review on a terminal destination was still
discarded so the move could not expose stale frozen output.

The current rule instead treats that snapshot as intentional retained session
state. `App::settle_terminal_focus` keeps an already-reviewed destination in
Normal/review, while a live Normal destination still starts Insert. This
avoids the original contradictory `[insert] [review]` state without erasing
the review: application mode and terminal-local state agree on Normal until an
explicit terminal insert key discards the snapshot. Coverage lives in
`tests/terminal.rs` tests
`control_w_focus_preserves_review_until_an_insert_key` and
`control_w_from_document_insert_preserves_terminal_review`.

## Report

After the integrated terminal's `Ctrl-w` motions were changed to keep a
terminal live while moving between panes, the behavior worked for one
terminal but could fail with several panes containing terminals. On affected
terminals, `Ctrl-w` navigation showed Normal/review state instead of keeping
terminal input live and moving to another pane.

The affected session was not consistently the second terminal. Its apparent
relationship to position in the pane grid varied, and the exact reproduction
required a more complex sequence than simply opening two terminals.
