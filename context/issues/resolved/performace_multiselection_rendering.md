---
title: "Rendering many selections approaches a full frame budget"
status: resolved
reported: 2026-09-28
resolved: 2026-09-28
commit: ba401d4
---

## Resolution

Commit `ba401d4` (`Avoid scanning offscreen selections during snapshot rendering`)
resolves the plain-text viewport cost. `App::snapshot_text_runs` in
`src/snapshot.rs` had searched every pane selection for a head and then walked
every range for each visible character. Its structured-table path also gathered
heads from every range before drawing each visual row. Offscreen selections
therefore contributed repeatedly to snapshot construction.

`SelectionRoles` now starts at the sorted ranges near the first visible offset
and advances separate head and coverage positions alongside visible text. It
keeps the original precedence of Replace carets, primary and secondary carets,
pristine one-character search matches, and selected text under inclusive and
half-open semantics. If a structured row revisits an earlier document offset,
the positions are located again. The table path limits its projected head
collection to the current logical row and walks those heads with visible atoms.
An open search prompt still draws its preview in place of the committed
selection; the preview's own indexed lookup is unchanged. Offscreen ranges
remain in the pane selection.

The same optimized 20,000-line fixture and 120×40 viewport described below,
with ten warmups and 100 samples per count, produced median snapshot times of
73.4 microseconds for 1 cursor, 78.4 microseconds for 100, 107.1 microseconds
for 1,000, and 478.0 microseconds for 10,000. The original 10,000-cursor
measurement was 13.7681 ms; these runs may differ in host conditions. The
measurement covers snapshot construction only.

Coverage lives in `src/snapshot.rs`:
`selection_role_walk_matches_range_semantics_at_boundaries_and_after_a_backward_jump`,
`offscreen_carets_do_not_change_visible_wrapped_rows_or_selection_state`, and
`wrapped_table_keeps_selection_roles_on_continuation_rows`. The existing
`pristine_search_hides_secondary_carets_until_the_selection_moves`,
`pending_replace_marks_every_selection_head_as_a_replace_caret`, and
`an_open_search_prompt_draws_the_matches_enter_would_select` also exercise
the retained roles. `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, and `cargo test` passed on Linux after the change.

Known limitation: exceptionally large structured-table logical rows still
scan heads across that row for each visual row. The plain-text benchmark does
not establish viewport-only cost for this case.

## Report

Snapshot construction with 10,000 cursors consumed most of a 16 ms frame
budget before frontend rendering or transport. This was a P1 performance
issue.

At commit `22fd664`, `App::snapshot_text_runs` in `src/snapshot.rs` determined
each visible character's role by repeatedly scanning the pane's selection
ranges. Caret detection and selection membership included offscreen ranges.
The work grew approximately with visible characters multiplied by the total
selection count, rather than only the ranges intersecting the view.

An exploratory measurement on 2026-09-28 used Linux x86-64, an AMD Ryzen AI 9
365, and `cargo build --release --locked --lib`. A plain-text fixture contained
20,000 copies of `"abcdefghijklmnopqrstuvwxyz ".repeat(3) + "\n"`, making
each line 82 characters including its newline. A `HeadlessEditor` held point
ranges at offsets `i * 82`, with primary index zero. Repeated 120×40 snapshots
after ten warmups, with 100 samples per selection count, produced these
medians:

| Cursors | Snapshot time |
| ---: | ---: |
| 1 | 76.7 microseconds |
| 100 | 205.5 microseconds |
| 1,000 | 1.5154 ms |
| 10,000 | 13.7681 ms |

To reproduce, create that fixture in temporary storage with
`HeadlessEditor::with_text_in`, install the ranges through
`set_active_selection(Selection::new(ranges, 0))`, and measure repeated
`snapshot(120, 40)` calls in an optimized build. These figures exclude key
dispatch, terminal rendering, transport, and language-server activity; they
do not measure Helix.

Expected rendering cost was proportional to sorted ranges intersecting the
viewport and visible characters, without scanning all selections per
character. Required behavior included primary and secondary caret roles,
inclusive and half-open selection semantics, pristine search results, Replace
mode, and wrapped or structured table rows. Offscreen ranges had to remain
part of the real selection. Validation called for comparing rendered roles
across those cases and measuring the same visible viewport with increasing
numbers of offscreen selections.
