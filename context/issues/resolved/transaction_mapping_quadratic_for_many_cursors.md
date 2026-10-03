---
title: "Transaction offset mapping scales quadratically with cursor count"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 4903c0f
---

## Resolution

Commit `4903c0f` (`perf(text): index cumulative offsets for multi-cursor transactions`).

Immutable transactions now retain cumulative character deltas alongside inserted-character counts. map_offset uses a binary partition over normalized change endpoints instead of summing every preceding change. Before and After associations keep their existing treatment of replacement boundaries and repeated insertions. This adds one machine-word delta per change and changes repeated endpoint lookup from linear to logarithmic time.

The eager metadata construction also accepts raw reversed or extreme endpoints
without panicking. Saturating bookkeeping is used only for the derived cache;
the original `Change` endpoints remain intact so the headless application
boundary can reject invalid transactions atomically. Valid document mappings
retain their exact deltas. This preserves the validation contract that predated
the offset index.

Coverage: indexed_offset_mapping_preserves_all_normalized_boundary_associations in src/text.rs compares normalized combinations against the former mapping algorithm, including Unicode inserts and inverse transactions, then verifies undo restores the source. many_cursor_offsets_map_through_one_large_transaction covers 20,000 cursor pairs. All 17 text tests pass. benchmarks/transaction_mapping.py compiles actual before/current source; the measured optimized median for 20,000 pairs was 205.47 ms before and 1.172 ms after, excluding transaction construction.

`invalid_transactions_are_rejected_atomically_without_history` in
`tests/headless_editor.rs` covers reversed raw changes, endpoints beyond both
signed and unsigned offset limits, and a cumulative delta beyond the signed
range. Rejection preserves text and undo history.

## Report

`Transaction::map_offset` walks the preceding changes for every offset it maps.
`Selection::map` maps both endpoints of every range, and ordinary multi-cursor
editing creates one change per range. Mapping a transaction with N disjoint
cursor edits therefore performs quadratic work, including for each pane and
remembered search region that follows the edit.

The cost can be reproduced independently of rope mutation by constructing
20,000 insertions at distinct offsets and mapping one caret through each. The
last caret scans the entire transaction, and earlier carets repeat its prefixes.
Inserted character counts are already cached, so the repeated work is traversing
the change list and adding deltas rather than counting replacement text.

Offset mapping should reuse immutable transaction metadata and locate the
relevant change without scanning all previous changes. Preserve the current
association behavior at replacement starts and ends, repeated insertions at the
same offset, overlapping-input normalization, and inverse transactions used by
undo. Unicode coordinates must continue to count characters rather than bytes.
