---
title: "An explorer does not report when its directory listing becomes stale"
status: resolved
reported: 2026-09-03
resolved: 2026-09-04
commit: 192e7c8
---

## Resolution

Commit 192e7c8 (`Report an explorer whose directory changed on disk`) extends
the host file monitor to explorers instead of adding a second mechanism beside
it. `FileObservationRequest` gained an `ObservationTarget`, so one worker
answers two questions about a watched path: `ObservationTarget::File` reads
bytes as before, and `ObservationTarget::Directory { show_hidden }` reads a
listing. `Buffer::file_observation_request` was returning `None` for every
kind but `BufferKind::File`; it now also registers a directory buffer, taking
`show_hidden` from the baseline snapshot the explorer accepted rather than
from the live preference, so a listing is always compared against one read
under the same dotfile choice.

The compared value is `fs_plan::DirectoryListing`: one name and kind per
listed entry, in listing order. It is deliberately not a `DirectorySnapshot`,
whose per-entry fingerprints include a child directory's own length and
modification time — values that move when that child's contents change while
the row in this explorer still names the same directory. `DirectorySnapshot::
read_with` and `DirectoryListing::read` now share `read_listed_rows`, the one
function that decides which entries a listing covers and which names the
editable explorer can represent, so a staleness check and a plan baseline
cannot disagree about what a directory contains.

`Buffer::apply_file_observation` was a single file-shaped body behind a
`kind != BufferKind::File` guard. It now checks path and generation first and
then dispatches to `apply_observed_file` or `apply_observed_directory`. The
directory arm compares the observed listing with `directory.baseline().
listing()` and sets `ExternalFileStatus::Changed` when they differ,
`Deleted` when the directory is gone, and `Unreadable` when the path is no
longer a directory or cannot be listed. It writes no text, no selection, and
no history: an explorer is editable, and replacing its rows would discard a
rename that has been typed but not yet written to a plan.

Clearing the marker is tied to accepting a fresh listing rather than to any
one command. `reload_directory`, `accept_directory_plan`, and
`retarget_directory` call the new `accept_current_as_listing_baseline`, and
`rebase_directory_after_external_removals` does the same work when it rebases.
Each advances `disk_generation`, which both clears the marker and retires
observations already in flight against the listing being replaced, so a
refresh cannot be immediately re-marked by the read that reported the change.

The cadence is the monitor's existing two-second reconciliation plus native
events. A directory has no cheap metadata baseline to compare ahead of
reading it — the accepted baseline is a listing, and a directory buffer has no
`DiskState` — so the reconciliation reads the listing every pass. To keep that
from waking the editor, the worker keeps the last listing it forwarded per
registration and sends nothing when the new one is identical; the record is
dropped whenever a registration changes, so a buffer that has accepted a new
baseline still hears the next observation. Native watches also changed shape:
a file is still watched through its parent directory, while an explorer is
watched directly, and `listing_affects` accepts only the directory itself and
its immediate children.

Entries excluded by the active dotfile filter were left undecided by the
report. They do not make a listing stale: `read_listed_rows` never reads them,
which keeps the existing contract that a listing read without dotfiles is
about the entries it showed.

Tests, all in `src/app/tests/navigation_and_files.rs` unless noted:

- `external_children_make_an_explorer_listing_stale_until_it_is_refreshed`
  covers external addition, removal, and rename of an immediate child, that
  the rows are not replaced, and that a refresh clears the marker and a later
  observation leaves it clear.
- `a_stale_explorer_marks_every_pane_and_keeps_its_unsaved_edits` covers
  propagation to every pane's title through the snapshot, and that the edited
  text, the dirty flag, and undo all survive.
- `an_observation_of_the_listing_being_replaced_cannot_re_mark_a_refreshed_explorer`
  covers the generation guard.
- `a_hidden_entry_outside_the_listing_does_not_make_it_stale` covers the
  dotfile decision above.
- `a_removed_directory_is_reported_without_discarding_the_explorer` covers the
  deleted directory.
- `an_explorer_hears_its_listing_once_and_then_stays_quiet` and
  `an_explorer_wakes_for_its_own_children_and_not_for_deeper_writes` in
  `src/file_monitor.rs` cover the worker: a directory registration produces a
  listing observation, an unchanged listing is not forwarded again across a
  reconciliation, and a write below a child directory does not wake the
  explorer.

Known limitation: staleness is about the rows a listing projects, so a change
that leaves every name and kind in place is not reported. With file details
shown, a child's size, mode, or modification time can therefore change outside
Runyte while the explorer's prefix columns keep showing the older values and
no marker appears; the details are presentation-only and the listing itself
still agrees. Removing or renaming the watched directory is likewise noticed
by the two-second reconciliation rather than immediately, because an explorer's
native watch is on the directory itself and not on its parent.

## Report

An explorer presents the directory listing accepted when the buffer was
opened, navigated, or refreshed. If another process changes that directory on
disk, the listing can stop agreeing with the directory without any indication
in the pane title.

### Reproduction

1. Open a directory as an explorer.
2. Outside Runyte, add, remove, or rename an entry directly below that
   directory.
3. Leave the explorer open without refreshing it.

The explorer continues to show its prior listing and its pane title remains
`[explorer] <path>`.

### Expected behavior

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
such as entries excluded by the active dotfile filter, was left undecided by
this report.

### Regression coverage

Cover external addition, removal, and rename of an immediate child;
propagation of the marker to every pane showing the explorer; preservation of
a dirty explorer while stale; and clearing the marker only after a fresh
directory listing is accepted.
