---
title: "Rendering diagnostic rows repeatedly scans all file diagnostics"
status: resolved
reported: 2026-09-28
resolved: 2026-09-28
commit: a789ac9
---

## Resolution

Commit `a789ac9` (`Index published diagnostics by row for rendering`) changed
`DiagnosticStore` in `src/lsp/diagnostics.rs` to index each publication by its
starting row. Previously, `for_row` filtered the complete per-file diagnostic
list and sorted the matches for every query. `severity_for_row` scanned that
same list to find the maximum severity. Constructing a frame repeats those
queries across visible rows, so diagnostics outside the viewport increased
frame work.

`FileDiagnostics::new` now retains the diagnostic vector in publication order
and builds a row map of indices into it. Each row's indices are stably sorted
by descending severity when the batch is published. `for_row` reads only the
requested row's indices, and `severity_for_row` reads the first index. Keeping
the original vector preserves the order and raw diagnostics used by the
workspace picker and code actions. Replacing a batch builds a fresh index;
empty publications, path clearing, and clearing the last publishing language
remove it with the old diagnostics. The row remains the diagnostic range's
start line. Encoding-aware range conversion and zero-width highlighting stay
in `App::diagnostic_spans`, where they use the current buffer text.

`benches/diagnostic_rows.rs` measures seven timed samples after a warmup in an
optimized build. It keeps a temporary 10,120-line file and 120×40 screen
geometry fixed, alternates the caret between columns 0 and 1, and times each
cursor motion, viewport preparation, and snapshot. Sparse cases place added
diagnostics on distinct offscreen rows; the dense case puts them on one visible
row. Each batch also contains three fixed visible diagnostics. Times below are
medians:

| Shape | Added | Insert | Replace | 40-row lookup | 40-row severity | 120×40 frame |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Sparse | 0 | 0.000 ms | 0.000 ms | 1.286 µs | 0.681 µs | 0.038 ms |
| Sparse | 100 | 0.002 ms | 0.004 ms | 0.940 µs | 0.735 µs | 0.038 ms |
| Sparse | 1,000 | 0.025 ms | 0.047 ms | 0.974 µs | 0.771 µs | 0.038 ms |
| Sparse | 10,000 | 0.361 ms | 0.637 ms | 1.017 µs | 0.810 µs | 0.039 ms |
| Dense | 10,000 | 0.018 ms | 0.214 ms | 5.493 µs | 0.687 µs | 2.706 ms |

Regression coverage is in `src/lsp/diagnostics.rs`:

- `row_index_tracks_publication_order_replacement_and_clearing` checks severity
  and equal-severity order, publication order, missing rows, counts, batch
  replacement, mixed-language ownership of a path, and path/language clearing.
- `a_row_reports_its_most_severe_diagnostic`,
  `publishing_an_empty_list_clears_a_file`, and
  `stopping_a_server_clears_only_its_own_files` cover row severity and clearing
  behavior.

`diagnostic_row_index_keeps_encoding_and_zero_width_presentation` and
`diagnostics_render_as_signs_spans_and_an_inline_message` in
`src/app/tests/language.rs` cover the editor's gutter severity, range spans,
inline text, UTF-16 conversion, zero-width highlighting, and publication
clearing. `diagnostics_render_a_sign_column_and_an_inline_message` in
`src/ui.rs` covers the rendered sign and inline message.

Known limitation: a visible row containing many diagnostics still costs more
to snapshot because each matching range is converted and considered for
highlighting. The case with 10,000 added visible diagnostics took 2.706 ms per
frame; its cost is proportional to the output on that row.

## Report

Priority: P3 (low). At commit `22fd664`,
`DiagnosticStore::for_row` in `src/lsp/diagnostics.rs` filtered the complete
per-file diagnostic list, allocated the matching rows, and sorted them by
severity. `severity_for_row` also scanned the full list. Frame construction
requested row diagnostics through `App::diagnostic_spans` in
`src/app/language_workflows.rs` and inline diagnostic presentation in
`src/snapshot.rs`. Repeated row lookups therefore scaled with both visible
rows and total file diagnostics, including diagnostics outside the view. The
2026-09-28 investigation found the scaling problem in code but did not
measure its latency on a representative diagnostic-heavy file.

Expected behavior: a row lookup should depend mainly on the diagnostics
associated with that row. A row index with severity ordering or aggregate
severity maintained at publication was a candidate fix. The interpretation
of a diagnostic's row, encoding-aware range conversion, zero-width
highlighting, ordering, and per-language clearing had to be preserved.

Reproduction: publish diagnostic sets of increasing size for a temporary
source file while keeping its visible text and viewport fixed, with most
diagnostics outside the viewport. After each set is installed, measure
repeated 120×40 cursor-only snapshots and independent row lookups in an
optimized build, comparing sparse and dense sets. Validation should check row
results and gutter severity after replacement, clearing, and mixed-severity
publications, including multiple languages for a path. Publication cost must
also be measured so indexing does not introduce a new input-loop stall on
large diagnostic batches.
