---
title: "Soft-wrapped continuation rows are not marked in the gutter"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: da3e1cd
---

## Resolution

Commit da3e1cd (`Mark soft-wrap continuation rows`) fixed `snapshot_line` in
`src/ui.rs`, which rendered the line-number and marker cells as blank spaces
for every wrapped continuation row. It now places `↪` in the existing gutter
marker cell. The marker uses the snapshot's authoritative `continuation`
state, so it does not recalculate wrapping in the frontend or consume a text
cell. It is deliberately different from the `▸` fold marker and remains in
the line-number area as requested.

Coverage is in `soft_wrap_marks_continuation_rows_in_the_line_number_gutter`
in `src/ui.rs`.

## Report

Soft-wrapped lines should be marked with an arrow beside the line number,
comparable to the marker used for folded regions but visually distinct. Helix
draws its equivalent in the text area; here it belongs in the line-number
gutter.
