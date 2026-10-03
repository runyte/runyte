---
title: "Language-server location rows overflow while building a picker"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: ee64775
---

## Resolution

Commit `ee64775` (`fix(lsp): widen location rows before formatting picker labels`).

Location picker labels now widen the zero-based protocol row to u64 before adding one. An extreme server coordinate can no longer panic debug builds or wrap to displayed row zero in release builds. Selecting an out-of-range location still uses the existing document-clamped navigation, and text mutation validation is unchanged.

Coverage: lsp_location_picker_displays_maximum_protocol_row_without_overflow in src/app/tests/language.rs reproduces the former panic, verifies row 4294967296, safely accepts the result, and checks unchanged document text. All 103 app language tests pass.

## Report

Language-server location results can crash the editor when a result uses the
largest representable protocol row. With at least two definition or reference
locations, the editor builds a picker and adds one to each zero-based `u32` row
before formatting it. A row of `4294967295` overflows: debug builds panic, while
release builds display row zero.

The response must remain harmless presentation data even when a server returns
an out-of-range position. The picker should display the one-based row without
overflow, and selecting that result should retain the existing document-bound
clamping used by navigation. Text mutation coordinates must retain their stricter
validation.

Reproduce by delivering a references response containing two local-file
locations, with one start position at line `4294967295`, character `0`, then
opening the resulting picker.
