# Explorer does not report when its directory listing becomes stale

An explorer presents the directory listing accepted when the buffer was opened,
navigated, or refreshed. If another process changes that directory on disk, the
listing can stop agreeing with the directory without any indication in the
pane title.

## Reproduction

1. Open a directory as an explorer.
2. Outside Runyte, add, remove, or rename an entry directly below that
   directory.
3. Leave the explorer open without refreshing it.

The explorer continues to show its prior listing and its pane title remains
`[explorer] <path>`.

## Expected behavior

Runyte should inspect open explorers periodically and detect when the
underlying directory content no longer agrees with the listing baseline that
the explorer accepted. Every pane showing an affected explorer should add
`[STALE]` to its title, following the existing marker used for externally
changed ordinary files.

Detection must not replace the explorer text automatically: selections,
unsaved edits, and undo history remain intact. An explicit refresh or another
operation that accepts a fresh listing clears `[STALE]` once the explorer and
directory agree again. The check should run at a bounded cadence rather than
performing filesystem work during each redraw.

At minimum, adding, removing, or renaming an immediate child must make the
listing stale. The treatment of changes that do not alter the projected rows,
such as entries excluded by the active dotfile filter, remains to be decided.

## Regression coverage

Cover external addition, removal, and rename of an immediate child; propagation
of the marker to every pane showing the explorer; preservation of a dirty
explorer while stale; and clearing the marker only after a fresh directory
listing is accepted.
