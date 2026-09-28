# P2 — Editing synchronously recomputes whole-document diffs before display

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
