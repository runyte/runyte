# P3 — Rendering diagnostic rows repeatedly scans all file diagnostics

Priority: P3 (low). The scaling problem is visible in code, but its latency
impact on a representative diagnostic-heavy file remains unmeasured.

At commit `22fd664`, `DiagnosticStore::for_row` in `src/lsp/diagnostics.rs` filters
the complete per-file diagnostic list, allocates the matching rows, and sorts
them by severity. `severity_for_row` also scans the full list. Frame
construction requests row diagnostics through `App::diagnostic_spans` in
`src/app/language_workflows.rs` and inline diagnostic presentation in
`src/snapshot.rs`. Repeated row lookups therefore scale with both visible rows
and the file's total diagnostic count, including diagnostics outside the view.

The expected behavior is for a row lookup to depend mainly on the diagnostics
associated with that row. A candidate fix is an index by row, with severity
ordering or aggregate severity maintained when diagnostic batches change.
Preserve the existing interpretation of a diagnostic's row, encoding-aware
range conversion, zero-width highlighting, ordering, and per-language clearing.

To reproduce, publish diagnostic sets of increasing size for a temporary source
file while holding its visible text and viewport fixed. Keep most diagnostics
outside the viewport. After each set is installed, measure repeated 120×40
cursor-only snapshots and row lookups independently, comparing sparse and dense
sets in an optimized build. The 2026-09-28 investigation inspected the code but
did not run this diagnostic-specific benchmark.

Validation should check row results and gutter severity after replacement,
clearing, and mixed-severity publications, including multiple languages for a
path. Benchmark publication cost as well as lookup cost so indexing does not
introduce a new input-loop stall on large diagnostic batches.
