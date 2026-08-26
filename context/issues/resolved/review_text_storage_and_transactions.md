---
title: "Text storage, selections, and transaction foundations required a focused hardening review"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: 1d842a8
---

## Resolution

Commit 1d842a8 (`Preserve primary selection direction when merging`) found a
normalization defect in `Selection::new`. When an overlapping primary range
sorted after another range, `Range::merge` retained the earlier non-primary
range's direction. The normalized selection still designated the right merged
region as primary, but its active caret could silently move to the opposite
edge.

Normalization now explicitly orients a merged union from the original primary
range whenever that range is absorbed. Later chained merges inherit that
orientation. The independent review requested a forward-primary control that
also crosses a later overlap; that coverage was added and the revision was
approved.

Coverage is in
`src/selection.rs::tests::an_absorbed_primary_range_keeps_its_reverse_direction`,
`src/selection.rs::tests::a_nested_primary_range_orients_the_merged_outer_span`,
`src/selection.rs::tests::a_primary_range_merged_first_keeps_its_direction`,
and
`src/selection.rs::tests::an_absorbed_forward_primary_reorients_and_keeps_the_chained_union`.
Run them with `cargo test --locked selection::tests::`.

## Report

Runyte's text storage, character-offset, selection, and transaction foundations
required a proactive hardening review. The primary scope was `src/text.rs`,
`src/selection.rs`, the transaction and undo machinery in `src/buffer.rs`, and
their direct callers and tests.

The audit checked character versus byte boundaries, normalized selections,
transaction ordering and composition, inversion, rollback, undo and redo
grouping, empty documents, document ends, overlapping multi-selections,
integer conversions, Unicode, and large inputs. It also established the
invariants at public boundaries and verified that buffer mutations continue to
pass through transactions.

Only confirmed implementation or test defects safely within this category were
to be changed. The work was not to add editing features or deliberate keymap
changes. Each distinct fix required focused regression or property-style
coverage, an independent post-implementation code review, targeted tests, and
the repository checks `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`.
