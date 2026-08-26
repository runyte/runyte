// SPDX-License-Identifier: MPL-2.0

//! Multi-range selections over character offsets.
//!
//! A [`Range`] is an anchor and a head, both character offsets, describing the
//! half-open span `[from, to)`. A range whose anchor equals its head is a bare
//! caret, which is how Normal mode represents a cursor that has not yet
//! extended a selection.
//!
//! A [`Selection`] holds one or more ranges and designates one of them primary.
//! It is always normalized: ranges are sorted, overlapping ranges are merged,
//! and the primary index continues to refer to the same region afterwards.

use crate::text::{Assoc, Offset, Transaction};

/// An anchored span of text. `head` is where the caret is drawn.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Range {
    pub anchor: Offset,
    pub head: Offset,
}

impl Range {
    pub fn new(anchor: Offset, head: Offset) -> Self {
        Self { anchor, head }
    }

    /// A collapsed range: a caret with no selected text.
    pub fn point(offset: Offset) -> Self {
        Self {
            anchor: offset,
            head: offset,
        }
    }

    pub fn from(&self) -> Offset {
        self.anchor.min(self.head)
    }

    pub fn to(&self) -> Offset {
        self.anchor.max(self.head)
    }

    pub fn len(&self) -> usize {
        self.to() - self.from()
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Swaps anchor and head without changing the covered span.
    pub fn flip(&self) -> Self {
        Self {
            anchor: self.head,
            head: self.anchor,
        }
    }

    /// Collapses to a caret at the head.
    pub fn collapse(&self) -> Self {
        Self::point(self.head)
    }

    /// Moves the head, keeping the anchor.
    pub fn extend_to(&self, head: Offset) -> Self {
        Self {
            anchor: self.anchor,
            head,
        }
    }

    /// Moves the whole range so both ends sit at `head`.
    pub fn move_to(&self, head: Offset) -> Self {
        Self::point(head)
    }

    pub fn contains(&self, offset: Offset) -> bool {
        offset >= self.from() && offset < self.to()
    }

    /// True when the two ranges share at least one character, or when a caret
    /// sits inside or at the edge of the other range.
    pub fn overlaps(&self, other: &Self) -> bool {
        let (a, b) = (self.from(), self.to());
        let (c, d) = (other.from(), other.to());
        if a == b || c == d {
            // Carets merge only when they coincide with the other span.
            return a.max(c) <= b.min(d);
        }
        a.max(c) < b.min(d)
    }

    /// Unions two ranges, keeping this range's direction.
    pub fn merge(&self, other: &Self) -> Self {
        let from = self.from().min(other.from());
        let to = self.to().max(other.to());
        if self.anchor <= self.head {
            Self {
                anchor: from,
                head: to,
            }
        } else {
            Self {
                anchor: to,
                head: from,
            }
        }
    }

    /// Maps both ends through a transaction.
    ///
    /// Both ends associate *after*, so that typing at a caret leaves a caret
    /// past the inserted text rather than a selection of what was just typed.
    /// Text inserted strictly inside a range still extends it, because an
    /// interior offset shifts the head without moving the anchor.
    pub fn map(&self, transaction: &Transaction) -> Self {
        Self {
            anchor: transaction.map_offset(self.anchor, Assoc::After),
            head: transaction.map_offset(self.head, Assoc::After),
        }
    }
}

/// One or more ranges with a designated primary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selection {
    ranges: Vec<Range>,
    primary: usize,
}

impl Default for Selection {
    fn default() -> Self {
        Self::point(0)
    }
}

impl Selection {
    /// Builds a normalized selection. An empty range list collapses to a caret
    /// at offset zero, because a selection is never allowed to be empty.
    pub fn new(ranges: Vec<Range>, primary: usize) -> Self {
        if ranges.is_empty() {
            return Self::point(0);
        }
        let primary = primary.min(ranges.len() - 1);

        let mut indexed: Vec<(usize, Range)> = ranges.into_iter().enumerate().collect();
        indexed.sort_by_key(|(index, range)| (range.from(), range.to(), *index));

        let mut merged: Vec<Range> = Vec::with_capacity(indexed.len());
        let mut primary_index = 0;
        for (original, range) in indexed {
            match merged.last_mut() {
                Some(previous) if previous.overlaps(&range) => {
                    *previous = previous.merge(&range);
                    if original == primary {
                        let (from, to) = (previous.from(), previous.to());
                        *previous = if range.anchor <= range.head {
                            Range::new(from, to)
                        } else {
                            Range::new(to, from)
                        };
                    }
                }
                _ => merged.push(range),
            }
            if original == primary {
                primary_index = merged.len() - 1;
            }
        }

        debug_assert!(!merged.is_empty());
        let primary_index = primary_index.min(merged.len() - 1);
        Self {
            ranges: merged,
            primary: primary_index,
        }
    }

    pub fn single(range: Range) -> Self {
        Self {
            ranges: vec![range],
            primary: 0,
        }
    }

    pub fn point(offset: Offset) -> Self {
        Self::single(Range::point(offset))
    }

    pub fn ranges(&self) -> &[Range] {
        &self.ranges
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn primary_index(&self) -> usize {
        self.primary
    }

    pub fn primary(&self) -> Range {
        self.ranges[self.primary]
    }

    pub fn set_primary(&mut self, index: usize) {
        self.primary = index.min(self.ranges.len() - 1);
    }

    /// Replaces the primary range in place, then renormalizes.
    pub fn with_primary(&self, range: Range) -> Self {
        let mut ranges = self.ranges.clone();
        ranges[self.primary] = range;
        Self::new(ranges, self.primary)
    }

    /// Applies a function to every range.
    pub fn transform(&self, mut f: impl FnMut(Range) -> Range) -> Self {
        let ranges = self.ranges.iter().map(|range| f(*range)).collect();
        Self::new(ranges, self.primary)
    }

    /// Applies a function that may expand one range into several.
    pub fn flat_transform(&self, mut f: impl FnMut(Range) -> Vec<Range>) -> Option<Self> {
        let mut ranges = Vec::new();
        let mut primary = 0;
        for (index, range) in self.ranges.iter().enumerate() {
            let produced = f(*range);
            if index == self.primary {
                primary = ranges.len();
            }
            ranges.extend(produced);
        }
        if ranges.is_empty() {
            return None;
        }
        let primary = primary.min(ranges.len() - 1);
        Some(Self::new(ranges, primary))
    }

    /// Retains ranges matching a predicate, keeping a valid primary.
    pub fn retain(&self, mut keep: impl FnMut(&Range) -> bool) -> Option<Self> {
        let mut ranges = Vec::new();
        let mut primary = 0;
        for (index, range) in self.ranges.iter().enumerate() {
            if keep(range) {
                if index <= self.primary {
                    primary = ranges.len();
                }
                ranges.push(*range);
            }
        }
        if ranges.is_empty() {
            return None;
        }
        let primary = primary.min(ranges.len() - 1);
        Some(Self::new(ranges, primary))
    }

    /// Maps every range through a transaction.
    pub fn map(&self, transaction: &Transaction) -> Self {
        let ranges = self
            .ranges
            .iter()
            .map(|range| range.map(transaction))
            .collect();
        Self::new(ranges, self.primary)
    }

    /// Drops every range except the primary.
    pub fn keep_primary(&self) -> Self {
        Self::single(self.primary())
    }

    /// Drops the primary range, keeping the rest.
    pub fn remove_primary(&self) -> Self {
        if self.ranges.len() == 1 {
            return self.clone();
        }
        let mut ranges = self.ranges.clone();
        ranges.remove(self.primary);
        let primary = self.primary.min(ranges.len() - 1);
        Self { ranges, primary }
    }

    /// Moves the primary designation forward or backward.
    pub fn rotate(&self, forward: bool) -> Self {
        let len = self.ranges.len();
        let primary = if forward {
            (self.primary + 1) % len
        } else {
            (self.primary + len - 1) % len
        };
        Self {
            ranges: self.ranges.clone(),
            primary,
        }
    }

    /// Collapses every range to a caret at its head.
    pub fn collapse(&self) -> Self {
        self.transform(|range| range.collapse())
    }

    /// Flips anchor and head in every range.
    pub fn flip(&self) -> Self {
        self.transform(|range| range.flip())
    }

    /// Adds a range and makes it primary.
    pub fn push(&self, range: Range) -> Self {
        let mut ranges = self.ranges.clone();
        ranges.push(range);
        let primary = ranges.len() - 1;
        Self::new(ranges, primary)
    }

    /// Builds a transaction that replaces each range with produced text.
    ///
    /// Returning `None` for a range leaves it untouched. Because every range is
    /// contributed to a single transaction, a multi-cursor edit is one undo
    /// step.
    pub fn change_by(&self, mut f: impl FnMut(&Range) -> Option<String>) -> Transaction {
        let changes = self
            .ranges
            .iter()
            .filter_map(|range| {
                f(range).map(|text| crate::text::Change::new(range.from(), range.to(), text))
            })
            .collect();
        Transaction::new(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Change;

    #[test]
    fn a_selection_is_never_empty() {
        let selection = Selection::new(Vec::new(), 0);
        assert_eq!(selection.len(), 1);
        assert_eq!(selection.primary(), Range::point(0));
    }

    #[test]
    fn ranges_are_sorted_and_overlaps_merged() {
        let selection = Selection::new(
            vec![Range::new(10, 14), Range::new(0, 4), Range::new(2, 6)],
            0,
        );
        assert_eq!(selection.len(), 2);
        assert_eq!(selection.ranges()[0], Range::new(0, 6));
        assert_eq!(selection.ranges()[1], Range::new(10, 14));
    }

    #[test]
    fn merging_preserves_the_primary_region() {
        let selection = Selection::new(vec![Range::new(0, 4), Range::new(10, 14)], 1);
        assert_eq!(selection.primary(), Range::new(10, 14));
    }

    #[test]
    fn nested_ranges_merge_into_the_outer_span() {
        let selection = Selection::new(vec![Range::new(0, 20), Range::new(5, 9)], 0);
        assert_eq!(selection.len(), 1);
        assert_eq!(selection.ranges()[0], Range::new(0, 20));
    }

    #[test]
    fn adjacent_ranges_do_not_merge() {
        let selection = Selection::new(vec![Range::new(0, 4), Range::new(4, 8)], 0);
        assert_eq!(selection.len(), 2);
    }

    #[test]
    fn direction_is_preserved_through_merging() {
        let selection = Selection::new(vec![Range::new(6, 2), Range::new(4, 8)], 0);
        assert_eq!(selection.len(), 1);
        let range = selection.ranges()[0];
        assert_eq!((range.from(), range.to()), (2, 8));
        assert!(range.anchor > range.head, "reversed direction is kept");
    }

    #[test]
    fn an_absorbed_primary_range_keeps_its_reverse_direction() {
        let selection = Selection::new(vec![Range::new(0, 10), Range::new(15, 5)], 1);

        assert_eq!(selection.primary(), Range::new(15, 0));
    }

    #[test]
    fn a_nested_primary_range_orients_the_merged_outer_span() {
        let selection = Selection::new(vec![Range::new(0, 20), Range::new(12, 8)], 1);

        assert_eq!(selection.primary(), Range::new(20, 0));
    }

    #[test]
    fn a_primary_range_merged_first_keeps_its_direction() {
        let selection = Selection::new(vec![Range::new(10, 0), Range::new(5, 15)], 0);

        assert_eq!(selection.primary(), Range::new(15, 0));
    }

    #[test]
    fn an_absorbed_forward_primary_reorients_and_keeps_the_chained_union() {
        let selection = Selection::new(
            vec![Range::new(10, 0), Range::new(5, 15), Range::new(20, 12)],
            1,
        );

        assert_eq!(selection.primary(), Range::new(0, 20));
    }

    #[test]
    fn multi_range_edits_form_a_single_transaction() {
        let selection = Selection::new(vec![Range::new(0, 1), Range::new(4, 5)], 0);
        let transaction = selection.change_by(|_| Some("Z".to_owned()));
        assert_eq!(transaction.changes().len(), 2);
    }

    #[test]
    fn selections_follow_the_text_through_a_transaction() {
        let selection = Selection::new(vec![Range::new(0, 1), Range::new(4, 5)], 1);
        let transaction = Transaction::new(vec![Change::new(0, 1, "XXX")]);
        let mapped = selection.map(&transaction);
        assert_eq!(mapped.ranges()[0], Range::new(0, 3));
        assert_eq!(mapped.ranges()[1], Range::new(6, 7));
        assert_eq!(mapped.primary(), Range::new(6, 7));
    }

    #[test]
    fn typing_at_a_caret_leaves_a_caret() {
        let selection = Selection::point(3);
        let transaction = Transaction::insert(3, "xx");
        let mapped = selection.map(&transaction);
        assert_eq!(mapped.primary(), Range::point(5));
        assert!(mapped.primary().is_empty());
    }

    #[test]
    fn text_inserted_inside_a_range_extends_it() {
        let selection = Selection::single(Range::new(2, 6));
        let transaction = Transaction::insert(4, "xx");
        let mapped = selection.map(&transaction);
        assert_eq!(mapped.primary(), Range::new(2, 8));
    }

    #[test]
    fn removing_the_primary_keeps_a_valid_selection() {
        let selection = Selection::new(vec![Range::new(0, 1), Range::new(4, 5)], 1);
        let reduced = selection.remove_primary();
        assert_eq!(reduced.len(), 1);
        assert_eq!(reduced.primary(), Range::new(0, 1));
    }

    #[test]
    fn removing_the_only_range_is_a_no_op() {
        let selection = Selection::point(3);
        assert_eq!(selection.remove_primary().len(), 1);
    }

    #[test]
    fn rotation_wraps_in_both_directions() {
        let selection = Selection::new(
            vec![Range::new(0, 1), Range::new(4, 5), Range::new(8, 9)],
            0,
        );
        assert_eq!(selection.rotate(true).primary_index(), 1);
        assert_eq!(selection.rotate(false).primary_index(), 2);
    }

    #[test]
    fn retain_keeps_the_nearest_primary() {
        let selection = Selection::new(
            vec![Range::new(0, 1), Range::new(4, 5), Range::new(8, 9)],
            2,
        );
        let kept = selection.retain(|range| range.from() != 4).unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept.primary(), Range::new(8, 9));
    }
}
