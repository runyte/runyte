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
