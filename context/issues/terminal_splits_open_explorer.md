# Splitting a terminal opens the covered buffer instead of the explorer

An integrated terminal is pane content rather than a buffer. The pane still
retains the buffer it was showing before the terminal was opened. Splitting
that pane clears the new pane's terminal, but otherwise clones the pane, so the
new active pane shows that retained buffer.

## Reproduction

1. Open a file buffer.
2. Run `:terminal` in that pane.
3. Create a vertical or horizontal split from the terminal with `Space w v/s`
   in Normal mode or `Ctrl-w v/s` in Terminal Insert.

The original pane keeps the terminal, while the new pane shows the file from
step 1.

## Expected behavior

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

## Constraints

- A terminal session remains visible in only one pane; the new pane must not
  inherit or move the terminal.
- The new pane owns its explorer independently, following the existing
  one-explorer-per-pane behavior.
- Terminal Insert splitting must still leave the child live without starting
  a review and must activate the new explorer in Normal mode.
- Splitting from an ordinary or special buffer must keep copying that buffer,
  its selection, and its view state as it does now.

## Regression coverage

Cover both split axes from a live Terminal Insert pane and from a terminal in
Normal/review mode. Assert that the original pane retains the same terminal,
the new active pane shows a working-directory explorer, and the terminal's live
or review state is unchanged. Retain coverage that ordinary file and explorer
splits continue to copy their source buffer.
