# Git-style network routing and readable commit timestamps

Status: completed, 2026-10-01.

## Rectangular ASCII follow-up

The network now uses one rectangular ASCII row per commit, with fixed columns
for surviving paths and persistent path colors. Branch labels follow captured
branch tips along first-parent paths and use the commit path's color; unknown
names remain explicit. Hashes display six characters while navigation retains
full OIDs. The Unicode renderer, glyph toggle, diagonal routing, and connector
rows were removed. The implementation record below describes the original
design; current behavior is documented in the user guide and UI vocabulary.

## Implementation record

The router lives in `src/git/network/routing.rs`. Each commit retains its node
line and connector lines; pending OIDs, ordered lanes, colors and the color
allocator continue in the page cursor. Adjacent swaps use an explicit crossing,
and only equal parent destinations join. First-parent paths continue where
possible, while an existing destination retains its identity/color when joined.
Completed paths leave a separating row before a disconnected path can reuse the
column. Unicode and ASCII rendering share the same geometry and color cells.

`src/app/git_network.rs` builds explicit document-row and commit-row maps and
colors graph cells by path. Connector ownership is used for refresh restoration,
not for Enter. Row/column selections preserve multiple ranges across redraws,
including in split panes. Detail return restores document coordinates even when
another pane changed the ASCII setting while detail was open. The shared commit
detail renderer uses the existing author-local `author_datetime` field.

The routing budget is 64 connector rows per commit and 16 lanes, including
transient expansion lanes. Layout admits each route once against its cost while
reserving space for the first-parent join. Existing parent lanes use a directed
horizontal connector if an extra expansion slot or diagonal-route budget is
unavailable; intermediate paths remain crossings. Omitted routes list abbreviated
parent OIDs in an `overflow parents` note, and full OIDs remain in detail. No
lane-number annotation depends on subsequent compaction. A single shared crossing cell uses the moving edge's color;
paths on either side keep their own colors. The four existing semantic color
roles and private snapshot protocol are sufficient; no protocol change is needed.

The dimensional bounds also bound additional text and spans: a 200-commit page
has at most 13,000 graph rows of 31 cells, or 403,000 cells. Three UTF-8 bytes per
graph glyph, 20 spaces of connector indentation, and a newline per row give a
conservative 1,482,000-byte graph/prefix allowance per page, separate from the
existing bounded commit metadata. At most one color span per cell plus two
metadata spans per commit gives 403,400 spans before coalescing. Only the displayed
page has buffer text and application spans; cached pages retain two-byte cells
and bounded line vectors. At 10,000 commits this is at most 40,300,000 cell bytes
per generation, plus line vectors and commit records. Atomic refresh can retain
two generations. Bundled frontends still receive viewport snapshots through the
existing 8 MiB transport bound, not whole graph pages.

Before the review corrections below, a local debug-build inspection rendered the first page of repository history.
Fifty fresh layouts of its 200 commits (10,000 layouts, with commit cloning)
took 20–48 ms in two runs and accumulated 287,100 bytes of cell payload. A stress
run retaining 10,000 copies of a dense nine-parent case took 1.26 seconds,
produced 49 connector rows per block, and used 37,720,000 bytes of graph vector
capacity, excluding commit records and allocator overhead. These are local
observations, not performance gates or platform-wide baselines. No idle or
startup graph work was introduced.

Initial implementation validation: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
and `cargo test` passed on Linux (4,360 tests passed, 41 ignored across the test
binaries). Full tests require an unrestricted local environment: sandboxed Unix
socket creation returned `Operation not permitted`. The canonical
`cargo llvm-cov --locked --workspace` run passed with 143,150 of 155,953 lines
covered (91.79%), above the unchanged 89% floor. The router itself had 98.80%
line coverage. Native macOS and Windows acceptance remains for CI.

Behavior coverage includes rendered-path decoding against exact ancestry in 100
small DAGs, merge/first-parent/shared-parent cases, crossings, root separation,
compaction colors, dense overflow and page replay in `src/git/network/tests.rs`.
`src/app/tests/git_network.rs` covers connector identity, refresh, split and
multi-range preservation, path roles, snapshot round-tripping, detail return,
and physical Enter through both log and network. `src/app/tests/git.rs` covers
formatted dates when an existing detail buffer is reused. The provider tests in
`tests/git_provider.rs` cover a merge at the 200th commit, continuation colors,
invalid color cursors, exact ancestry, and author timezone boundaries.

Review corrections remove retained commit-line indices and obsolete active-lane
arrays. Network and diff reprojection share the text-replacement/selection loop;
network activation explicitly follows the new caret while redraws preserve scroll.
Detail return stores one row/column selection plus viewport state. A shared
metadata prefix defines graph offsets for both commit and connector rows, and
`HEAD ->` stays unchanged across glyph modes. Color choice uses a four-bit mask
without allocating. Layout no longer retries smaller parent lists. Regression
tests cover scrolled paging, moved refresh anchors, saturated lanes in both
directions, later existing parents after an omitted new parent, and exact HEAD
subject selections across toggles. Review validation passed on Linux:
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`
(4,366 passed, 41 ignored). The fresh canonical
`cargo llvm-cov --locked --workspace` result is 143,251 of 156,031 lines covered
(91.81%), above the unchanged 89% floor. Native macOS/Windows checks remain for
CI.

The remaining sections preserve the approved design; current behavior is
specified by the user guide and reference registers.

## Outcome

Make the commit network readable through continuous paths, consistent path
colors, and separate connector rows where forks, joins, or lane compaction need
space. Keep the compact commit metadata: hash, author initials, graph, captured
ref labels, and subject. In commit detail opened from either the network or
`Space g l`, display `Author-time: YYYY-MM-DD HH:MM` in the commit's original
timezone.

This revises the one-document-row-per-commit and stable-column decisions in
`PLAN_GIT_MERGES_CONFLICTS_AND_NETWORK.md`. Stable captured history and
repeatable cached pages remain requirements; individual paths may move between
columns. The completed plan remains historical evidence. Current behavior
registers change with the implementation.

## Diagnosis

`src/git/network.rs` currently assigns pending parent OIDs to sparse columns.
`GraphRow::graph_with_width` draws all parent connections horizontally on the
commit row, using double-stroke crossings to distinguish passing paths from
junctions. Several parent edges can share the same horizontal stroke. Empty
columns remain available for reuse rather than collapsing the graph.

`App::show_network_page` in `src/app/git_network.rs` assigns a repeating color
to each column. A horizontal connection therefore changes color as it crosses
columns, although those changes have no ancestry meaning. Together these choices
make correct parent relationships difficult to follow.

`selected_network_oid` indexes the commit array using document row minus one.
Selection restoration similarly adds one to a commit index. Connector rows
require explicit identity maps instead of this arithmetic.

`App::open_git_commit_detail_result` in `src/app/git_workflows.rs` renders
`CommitSummary::author_time`, producing text such as `Author-time: 1790767961`.
The same summary already contains validated `author_datetime` in the requested
format. The Git log uses that field today; both detail entry points share the
same renderer.

## Presentation and interaction contract

Commit lines retain the current column order. Connector lines leave the hash,
initials, labels, and subject areas empty and draw only the graph. Simple linear
history needs no extra rows. For example, this schematic shows a merge and its
shared ancestor; letters stand for full commit identities:

```text
M  AB  ●    Merge feature
       │╲
A  AB  ● │  Main-line change
B  CD  │ ●  Feature change
       │╱
R  AB  ●    Shared ancestor
```

Use downward vertical and diagonal paths to expand merges and collapse finished
lanes. Preserve first-parent continuation wherever possible. An existing parent
lane is joined explicitly rather than overwriting or duplicating its ancestry.
The graph remains newest-to-oldest in the existing topological order. Branch
names decorate commits; colors identify active paths, and a branch is not
promised one fixed screen column throughout history. No branch name is hardcoded
as the repository's mainline.

Each path keeps its assigned color when it bends or changes columns. A newly
introduced secondary-parent path gets a deterministic color, avoiding adjacent
active colors when the palette permits. On convergence, the surviving parent
path retains its color and the incoming path ends at the junction. Palette reuse
is allowed, but never changes an uninterrupted active path's color. Shapes must
remain understandable without color. Reuse the four existing `GitLane` theme
roles initially; select roles by path identity rather than horizontal position.

ASCII mode uses `*`, `|`, `/`, `\`, and necessary horizontal/junction glyphs;
Unicode mode uses matching one-cell nodes and connectors. Both derive from the
same topology and row schedule. Dense layouts may need unavoidable crossings;
crossings and joins must remain distinguishable, with additional routing rows
used to avoid an ambiguous shared stroke. Exact glyph-for-glyph imitation of a
particular Git version is not an acceptance requirement.

Connector rows are ordinary read-only document text. Normal movement, search,
selection, copying, and line numbers continue to operate on document rows;
`j` and `k` are not silently changed into commit navigation. Enter opens a commit
only on its metadata row. On a connector, header, or limit note it gives a short
action message directing the user to a commit row. It never guesses a nearby
commit. No new binding is needed.

Closing detail restores the originating page, caret, and viewport. Refresh keeps
the selected commit OID when reachable. If refresh begins on a connector, use
its owning commit block only as the refresh anchor, retaining the relative row
where possible; this ownership does not make Enter on a connector actionable.
ASCII toggling preserves the same row identity. Splits retain their independent
view positions, and delayed responses keep the existing focus/selection guards.

## Layout and data ownership

Keep machine-delimited Git records and the existing captured-root traversal in
`src/git/cli/network.rs`. Implement routing in the pure `src/git/network.rs`
module, extracting focused submodules if needed. Do not parse colored terminal
output or copy Git implementation code into the MPL-2.0 source tree.

Separate three concepts that currently share `GraphRow`:

- A commit record and its exact ordered parent OIDs.
- A logical commit block: one commit line followed by zero or more routing lines.
- Rendered cells/spans with glyph connectivity and path/color identity.

Keep incoming and outgoing lane order, pending destination OIDs, path identities,
and color allocation in layout continuation state. Process one commit by
locating its incoming path, assigning ordered parent destinations, expanding
secondary-parent paths, joining existing destinations, and compacting vacant
lanes. Emit a deterministic bounded sequence of routing rows. Preserve the first
parent's path identity where it continues; resolving an already active shared
parent must not recolor that existing path. Define junction and crossing cell
ownership explicitly so a single terminal cell never claims two independent
colors.

Generate graph spans from this output in `show_network_page`, replacing the
column-modulo-four loop. Build both document-row-to-kind/OID and OID-to-commit-row
maps at the same time as text and character-offset spans. Connector entries also
record their owning block and relative row for restoration. Use these maps for
Enter, refresh anchoring, selected-commit placement, and ASCII toggling. Preserve
Unicode initials, labels, subjects, nonwrapping rows, and character-based offsets.

Layout remains below the application and frontend boundary. Frontends consume
owned text and semantic color spans, including through persistent-session
snapshots. Reusing current text roles should require no new bundled protocol
value or public plugin contract. Verify existing snapshot/wire size bounds with
the expanded pages; if a wire shape must change, version the private protocol
explicitly rather than making an implicit compatibility change.

## Pagination and resource bounds

Retain 200 commits per page, 50 retained pages, 10,000 commits per generation,
16 active lanes, 256 captured refs plus HEAD, and the existing 2 MiB Git-read
output bound. A page counts commits rather than rendered lines. Complete all
routing rows for the final commit on that page before saving outgoing layout
state. The next page starts with exactly that lane order and color assignment.
Cached backward navigation restores identical text and spans. Root OIDs,
decorations, shallow fingerprints, stale detection, and atomic refresh behavior
retain their current ownership.

Additional lines and semantic spans need separate explicit allocation bounds.
In the routing prototype, establish and test a worst-case connector-row bound
derived from the 16-lane limit, then set per-block, per-page, text-byte, and span
budgets before integrating the layout. Include the existing maximum of two
generations during refresh in retained-memory accounting. No unbounded search
for a prettier layout or full-repository materialization is allowed.

If lanes or routing budgets cannot represent an edge, retain its exact parent
identity and display an explicit graph-limit/parent-inspection note. Never drop
ancestry silently, draw a false join, or label an unfinished path as a root.
Cover shallow boundaries, disconnected roots, and traversal-limit endings.
Rendering and toggling cached pages must not invoke Git. Layout runs on demand
with the existing Git work; add no startup work, timers, or background graph scan.

## Commit timestamps

Replace the detail renderer's use of `author_time` with `author_datetime`,
retaining the `Author-time:` label. Match the existing log's author timezone,
24-hour clock, and minute precision. Do not reinterpret the value in UTC or the
viewer machine's local timezone. Keep the numeric timestamp available in the
model for non-display uses. No new date library, Git subprocess, or parser field
is necessary.

The shared renderer covers network and log Enter paths and other consumers of
the same detail buffer. Verify both a newly created detail buffer and reuse of an
existing detail buffer, with intact parent metadata, body, patch boundary, and
network return navigation.

## Implementation sequence

1. Change the shared commit-detail timestamp and add focused regression coverage
   for both requested entry points. This can land independently of graph work.
2. Build and test the pure path router and its bounded continuation state.
   Establish the row/byte/span budgets and review deterministic small-history
   examples, including geometry and color expectations, before UI integration.
3. Integrate commit blocks, semantic spans, document identity maps, page cursors,
   and restoration in the network buffer. Update any provider test doubles and
   cursor validation affected by the richer state.
4. Validate rendered behavior and persistent snapshots, then update the user
   guide, the commit-network section of `context/reference/ui-vocabulary.md`,
   and the network row in `context/reference/helix-keymap-v1.md`. Replace claims
   of one commit per document row and stable columns with the new contracts.
   Keep help and key hints driven by the registry. Move this plan through the
   repository lifecycle as implementation is approved and completed.

## Acceptance and validation

Pure routing tests must check parent reachability and path continuity as well as
readable expected diagrams and colors. Cover linear history, two-parent merges,
diverge/rejoin, a parent already pending on another lane, octopus merges,
criss-cross merges, disconnected roots, lane reuse, compaction, unavoidable
crossings, shallow history, and overflow. Assert no route gains a spurious parent
at a crossing and no color changes merely because a path changes column. Test
maximum-width expansion/compaction against the explicit budgets.

Extend `tests/git_provider.rs` network fixtures to compare represented ancestry
against exact commit parent OIDs. Check merge blocks at page boundaries, color
and lane continuation, replay after moving/deleting refs, SHA-256 OIDs, and
bounded failures. Existing tests such as
`network_pages_keep_root_objects_labels_and_lanes_when_refs_move` and
`network_octopus_and_criss_cross_ancestry_match_full_parent_graph` remain relevant
behavior boundaries, with assertions adapted to the new representation.

Extend `src/app/tests/git_network.rs` for Enter on commit and connector rows,
OID restoration with unequal block heights, refresh from a connector, page
navigation, ASCII toggling, split views, search-selected rows, delayed responses,
detail return, and semantic text/span round-tripping. Update
`graph_snapshot_roles_and_private_protocol_round_trip_preserve_owned_text` to
assert path colors through bends instead of colors assigned by column.

Add detail-format assertions in `src/app/tests/git.rs` and the network Enter
test. Reuse the provider's timezone-boundary coverage in `tests/git_provider.rs`
to prove the displayed value is the author-local formatted field, including a
case whose UTC date differs. Assert the exact 16-character date/time and absence
of the raw timestamp in the `Author-time:` field.

Review deterministic dense-history renders in Unicode, ASCII, and monochrome,
including narrow panes and horizontal scrolling. Compare the topology and
readability with local `git log --graph --oneline` for the same fixture; Git's
exact layout is not a golden file. Keep any durable examples in tracked test
fixtures, independent of disposable runtime image caches. Measure demand-driven
page layout and retained memory at the graph bounds; verify no idle work was
added.

Before handing off Rust implementation, run `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`. Run canonical
`cargo llvm-cov --locked --workspace` acceptance and preserve the coverage floor
defined in `context/reference/test-coverage.md` on Linux and macOS; retain native
Windows build/lint/test acceptance. Storage and subprocess fixtures use owned
temporary storage, including `XDG_CONFIG_HOME` for launched editors/hosts.
