---
title: "Splitting a terminal opens the covered buffer instead of the explorer"
status: resolved
reported: 2026-09-03
resolved: 2026-09-03
commit: 66a7acf
---

## Resolution

Commit `66a7acf` (`Open an explorer when a split comes from a terminal`)
changed `App::split` in `src/app/file_workflows.rs`.

`split` builds the new pane by cloning the source pane and then clearing the
few fields that must not be shared: the syntax history, the directory buffer,
`terminal`, and `covered_terminal`. Clearing `terminal` is what keeps one
pseudoterminal in one pane, but it left the clone's `buffer` untouched, and a
pane showing a terminal still points at whatever buffer it was showing before
the terminal opened. That buffer is the pane's history rather than a document
anyone asked for a second view of, so the new pane came up on it.

The fix reads `self.terminal_of_pane(old).is_some()` before the clone and, when
no path was given, calls `open_explorer(None)` after `activate_pane(new)`. The
question is asked of the source pane's live content, not of `covered_terminal`,
so a pane whose terminal is currently covered by a host-opened wait document
still splits that document as it did before. `open_explorer(None)` is the same
entry point `Space E` uses, so the new pane is rooted at the working directory
controlled by `:cd` and owns an explorer of its own through the existing
`pane.directory_buffer = None` rule. The call sits before the `self.status(...)`
line so the split still reports `vertical split` or `horizontal split`; a split
given an explicit path keeps opening that path instead.

Every command route funnels into this one function. Normal-mode `Space w v/s`
and `Ctrl-w v/s` and Terminal Insert `Ctrl-w v/s` all reach
`split_from_terminal_insert` from the single `EditorCommand::SplitVertical` /
`SplitHorizontal` arm in `App::execute_editor_command`, so no route needed a
separate change. `open_explorer` opens through `open_file`, which leaves the
new pane in Normal mode; `finish_insert_window_command` then finds no terminal
in the active pane and its `enter_normal_mode` call is a no-op, so the Terminal
Insert route neither captures a review snapshot nor disturbs the live child.

Tests:

- `splitting_a_terminal_opens_a_working_directory_explorer` in
  `src/app/tests/navigation_and_files.rs` covers both axes from a terminal in
  Normal/review mode, asserting that the source pane keeps the same terminal,
  that the new pane is not the covered buffer but a directory buffer at the
  working directory, and that the child stays live and still reviewing.
- `control_w_splits_an_only_terminal_without_starting_review` in
  `tests/terminal.rs` covers both axes from Terminal Insert over a real
  pseudoterminal, and now also asserts the retained terminal in the source pane
  and the working-directory explorer in the new one.
- `a_split_does_not_inherit_the_claim_on_a_covered_terminal` in
  `src/app/tests/navigation_and_files.rs` retains coverage that a pane whose
  terminal is covered splits its document.
- `splitting_an_explorer_shows_the_same_listing_in_both_panes` in
  `src/app/tests/navigation_and_files.rs` retains coverage that ordinary
  explorer and file splits keep copying their source buffer.

Known limitation: `open_explorer` canonicalizes the working directory and fails
if it is gone. A split made from a terminal whose editor working directory has
been deleted therefore reports that failure and leaves the new pane on the
retained buffer, which is the pre-existing behavior of `:vsplit <path>` when the
path cannot be opened.

## Report

An integrated terminal is pane content rather than a buffer. The pane still
retains the buffer it was showing before the terminal was opened. Splitting
that pane clears the new pane's terminal, but otherwise clones the pane, so the
new active pane shows that retained buffer.

### Reproduction

1. Open a file buffer.
2. Run `:terminal` in that pane.
3. Create a vertical or horizontal split from the terminal with `Space w v/s`
   in Normal mode or `Ctrl-w v/s` in Terminal Insert.

The original pane keeps the terminal, while the new pane shows the file from
step 1.

### Expected behavior

When a split originates from a pane currently showing an integrated terminal,
the new active pane shows the editable explorer rooted at the editor working
directory, matching `Space E`. This applies to both vertical and horizontal
splits and to every command route that invokes them. The terminal stays in the
original pane and keeps its live or review state.

When a split originates from a buffer, the existing behavior is unchanged:
the new pane shows another view of the current buffer. In particular, splitting
an explorer continues to show the same directory listing in both panes until
one of them navigates elsewhere.

The decision is based on what the source pane is showing when the split is
requested, not on the buffer retained behind a terminal.

### Constraints

- A terminal session remains visible in only one pane; the new pane must not
  inherit or move the terminal.
- The new pane owns its explorer independently, following the existing
  one-explorer-per-pane behavior.
- Terminal Insert splitting must still leave the child live without starting
  a review and must activate the new explorer in Normal mode.
- Splitting from an ordinary or special buffer must keep copying that buffer,
  its selection, and its view state as it does now.

### Regression coverage

Cover both split axes from a live Terminal Insert pane and from a terminal in
Normal/review mode. Assert that the original pane retains the same terminal,
the new active pane shows a working-directory explorer, and the terminal's live
or review state is unchanged. Retain coverage that ordinary file and explorer
splits continue to copy their source buffer.

The report did not say what should happen when a split is given an explicit
path from a terminal pane; the fix keeps opening that path.
