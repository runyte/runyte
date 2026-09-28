---
title: "Editing recomputes whole-document diffs before displaying the changed frame"
status: resolved
reported: 2026-09-28
resolved: 2026-09-28
commit: 5681520
---

## Resolution

Commit 5681520 (Compute large edit diffs off the frame path) resolved this.
GitTracker::update and App::prepare_diffs were copying and aligning complete
texts during frame preparation whenever a buffer revision changed. The edited
frame therefore waited for comparison cost proportional to document size.

Large Git gutter and side-by-side comparisons now run on a lazy worker with
coalesced requests and results. Pending marks clear until a result for the
current buffer revision and staged-base identity arrives; stale results cannot
replace newer marks. Both views continue to use the shared line-alignment
implementation for their exact results. Small comparisons remain synchronous.

A side-by-side comparison needs row correspondence before the worker finishes.
DiffSession retains the previous alignment for character-only edits and splices
its row runs for inserted or deleted newlines, preserving known correspondence
outside the changed ranges. Undo and redo replay their ordered history
transactions against a captured rope and apply those same local row changes.
The worker's exact alignment replaces this provisional geometry only when its
session, panes, buffers, and revisions still match. The user guide describes
the brief pending state; measured edit-to-frame and worker costs are recorded
in context/reference/startup-performance.md.

Regression coverage is provided by:

- large_live_comparison_keeps_far_row_mapping_while_edits_are_in_flight,
  large_comparison_character_undo_and_redo_keep_known_row_mapping, and
  distant_multi_range_undo_and_redo_keep_middle_correspondence in
  src/app/tests/comparisons.rs;
- provisional_splices_keep_unchanged_correspondence_below_a_change and
  provisional_splices_preserve_each_remaining_row_for_both_sides in src/diff.rs;
- multi_range_newline_edit_preserves_correspondence_below_both_changes in
  src/diff_view.rs; and
- delayed_marks_reject_superseded_edits_and_refreshed_bases in
  src/git/tracker.rs.

The ignored large_diff_edit_to_frame_latency and
large_diff_history_to_frame_latency tests in src/app/tests/diff_latency.rs, and
large_diff_worker_cost in src/app/diff_work.rs, reproduce the release timing
measurements.

Known limitation: opening a side-by-side comparison still computes its initial
alignment synchronously. A whole-buffer replacement outside the transaction
path may temporarily align rows by number until the exact result arrives.
Undoing back to saved text can still pay Buffer::update_dirty's full-content
equality check; this is separate from diff computation. Exact worker cost also
grows if an open buffer is edited beyond the 4 MiB comparison-opening limit.

## Report

Priority: P2 (normal). The cost grows with document size and is paid before the
edited frame appears; ordinary movement without edits already skips it.

At commit `22fd664`, `App::prepare_view` calls `update_git_marks` in
`src/app/git_workflows.rs`. When a tracked buffer's revision changes,
`GitTracker::update` in `src/git/tracker.rs` obtains a complete string copy and
runs `changed_rows`, which performs full-text line alignment. No subprocess is
needed, but the comparison remains synchronous on the frame preparation path.
`App::prepare_diffs` in `src/app/presentation.rs` similarly copies both sides
and recomputes live comparison alignment when either revision changes.

An exploratory measurement on 2026-09-28 used Linux x86-64, an AMD Ryzen AI 9 365,
and `cargo build --release --locked --lib`. Fixtures contained lines of the
form `local function item_{i}(x) return x + {i} end\n`. The current text differed
from the staged base by one leading space. After ten warmups, 50 calls to
`GitTracker::update` with distinct revisions produced these medians:

| Lines | Update time |
| ---: | ---: |
| 2,000 | 36.7 microseconds |
| 20,000 | 398.9 microseconds |
| 70,000 | 2.5208 ms |

To reproduce, install each base using `apply_staged_content` and supply a clone
of the modified string to `update` while incrementing its revision. This
measures the string copy and gutter computation; it does not measure rope
flattening, the complete edit/frame path, or live side-by-side diff latency.
The latter path was identified by inspection only. The existing 4 MiB staged
base limit and alignment complexity bound remain relevant constraints.

Small edits should not require an immediate full-document comparison before
the editor can acknowledge input. Candidate approaches include asynchronous,
coalesced gutter computation or incremental updates. Results must be tied to
the exact document and base revisions so stale work cannot overwrite newer
marks. Live comparison alignment additionally controls viewport geometry and
needs explicit consistency rules if deferred.

Validation should cover rapid edits, undo/redo, base refresh, deletion of lines,
buffer closure, and superseded computations. Git gutters and side-by-side views
must continue to use the shared alignment implementation and agree about
changes. Measure complete edit-to-frame latency as well as worker cost.
