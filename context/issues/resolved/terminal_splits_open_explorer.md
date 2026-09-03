---
title: "Splitting a terminal opens the covered buffer instead of the explorer"
status: resolved
reported: 2026-09-03
resolved: 2026-09-03
commit: 66a7acf
---

## Resolution

Commit `66a7acf` (`Open an explorer when a split comes from a terminal`)
changed `src/app/file_workflows.rs`.

`App::split` builds the new pane by cloning the source pane and then clearing
the few fields that must not be shared: the syntax history, the directory
buffer, `terminal`, and `covered_terminal`. Clearing `terminal` is what keeps
one pseudoterminal in one pane, but it left the clone's `buffer` untouched, and
a pane showing a terminal still points at whatever buffer it was showing before
the terminal opened. That buffer is the pane's history rather than a document
anyone asked for a second view of, so the new pane came up on it.

The decision is made in `App::split_window`, the entry point the split commands
reach, not in `split` itself. It reads `self.active_terminal().is_some()`
before splitting and, when the source pane was showing a terminal, calls
`open_explorer(None)` in the new pane afterwards. `split` stays the pane
primitive that clones and nothing more, because `diff_sides`, the tutorial's
`create_tutorial_view`, and `open_git_file_comparison_result` all split in
order to retarget both resulting panes themselves and need the plain copy.
Putting the explorer inside `split` broke `:diff-this` from a terminal pane:
`diff_sides` records `DiffSide { pane: opened, buffer: active }` on the
assumption that the new pane still shows the source pane's buffer, and
`prepare_diffs` drops any session whose recorded buffer no longer matches its
pane, so the comparison was reported on the status line and then silently
discarded on the next frame.

The question is asked of the source pane's live terminal, not of
`covered_terminal`, so a pane whose terminal is currently covered by a
host-opened wait document still splits that document as it did before.
`open_explorer(None)` is the same entry point `Space E` uses, so the new pane
is rooted at the working directory controlled by `:cd` and owns an explorer of
its own through the existing `pane.directory_buffer = None` rule. `open_file`
reports `opened <path>`, so `split_window` restates the split's own status
afterwards; the shared wording moved into the free function `split_status`. A
split given an explicit path still opens that path, through `split` directly.

Every split command route funnels into `split_window`. Normal-mode
`Space w v/s` and `Ctrl-w v/s` and Terminal Insert `Ctrl-w v/s` all reach it
from the single `EditorCommand::SplitVertical` / `SplitHorizontal` arm in
`App::execute_editor_command`, and `:vsplit`/`:hsplit` without a path fall back
to that same arm. `open_explorer` opens through `open_file`, which does not
change the mode; the one `enter_normal_mode` comes from
`finish_insert_window_command` after `split_window` returns, by which point the
active pane is the new explorer and holds no terminal, so the Terminal Insert
route neither captures a review snapshot nor disturbs the live child.

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
- `comparing_from_a_terminal_pane_still_opens_the_view` in
  `src/app/tests/comparisons.rs` covers `:diff-this` from a terminal pane,
  which is the caller that needs `split` to hand back the plain pane copy.
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
path cannot be opened. Because the call is in `split_window` rather than in
`split`, that failure cannot reach the tutorial or the Git comparison, which
split without a path for reasons of their own.

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
