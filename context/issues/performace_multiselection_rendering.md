# P1 — Rendering many selections approaches a full frame budget

Priority: P1 (high). Measured snapshot construction with 10,000 cursors consumes
most of a 16 ms frame budget before frontend rendering or transport.

At commit `22fd664`, `App::snapshot_text_runs` in `src/snapshot.rs` determines
each visible character's role by repeatedly scanning the pane's selection
ranges. Caret detection and selection membership include offscreen ranges.
The work therefore grows approximately with visible characters multiplied by
the total selection count, rather than only the ranges intersecting the view.

An exploratory measurement on 2026-09-28 used Linux x86-64, an AMD Ryzen AI 9 365,
and `cargo build --release --locked --lib`. A plain-text fixture contained
20,000 copies of `"abcdefghijklmnopqrstuvwxyz ".repeat(3) + "\n"`, making each
line 82 characters including its newline. A `HeadlessEditor` held point ranges
at offsets `i * 82`, with primary index zero. Repeated 120×40 snapshots after
ten warmups, with 100 samples per selection count, produced these medians:

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

Rendering should locate the sorted ranges intersecting the viewport and walk
them alongside visible characters, without scanning all selections per
character. Preserve primary and secondary caret roles, inclusive and half-open
selection semantics, pristine search results, Replace mode, and wrapped or
structured table rows. Offscreen ranges must remain part of the real selection.

Validation should compare rendered roles across these cases and measure the
same visible viewport with increasing numbers of offscreen selections.
