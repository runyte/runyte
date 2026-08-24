// SPDX-License-Identifier: MPL-2.0

//! Rope-backed text storage, coordinates, and transactional edits.
//!
//! Character offsets are the single internal coordinate. [`Position`] is a
//! derived view coordinate: it exists so rendering and user-facing commands can
//! speak in rows and columns, and it is converted at that boundary rather than
//! stored.
//!
//! Every mutation of buffer text goes through a [`Transaction`]. Applying one
//! returns a [`Revert`], which is the inverse transaction. Undo history is a
//! stack of those inverses, so its memory cost is proportional to the size of
//! each edit rather than to the size of the document.
//!
//! Ropey is configured for LF-only line breaks so that line indexing matches
//! the `split('\n')` semantics the durable formats and tests already assume.

use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

use ropey::{Rope, RopeSlice};

/// A character offset into a document.
pub type Offset = usize;

/// A derived view coordinate. `col` is a character index within the row, not a
/// byte index and not a display column.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Position {
    pub row: usize,
    pub col: usize,
}

impl Position {
    pub fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }
}

/// Which side of a replaced region an offset should land on when it is mapped
/// through a transaction that deleted the text it pointed into.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Assoc {
    /// Collapse to the start of the replacement.
    Before,
    /// Collapse to the end of the replacement.
    After,
}

/// Replacement of the half-open character range `[from, to)` with `text`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Change {
    pub from: Offset,
    pub to: Offset,
    pub text: String,
}

impl Change {
    pub fn new(from: Offset, to: Offset, text: impl Into<String>) -> Self {
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        Self {
            from,
            to,
            text: text.into(),
        }
    }

    fn inserted_len(&self) -> usize {
        self.text.chars().count()
    }

    fn removed_len(&self) -> usize {
        self.to - self.from
    }

    fn delta(&self) -> isize {
        self.inserted_len() as isize - self.removed_len() as isize
    }
}

/// An ordered set of non-overlapping changes applied as a single unit.
///
/// A transaction is the only way to mutate buffer text, and it is also the unit
/// of undo: a multi-cursor edit touching twenty ranges is one transaction and
/// therefore one undo step.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Transaction {
    changes: Vec<Change>,
}

/// The inverse of an applied [`Transaction`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Revert {
    changes: Vec<Change>,
}

impl Revert {
    pub fn into_transaction(self) -> Transaction {
        Transaction {
            changes: self.changes,
        }
    }
}

impl Transaction {
    /// Builds a transaction from arbitrary changes.
    ///
    /// Changes are sorted by position. Overlapping changes are merged by
    /// dropping the later one, because two cursors editing the same region is a
    /// selection-model bug rather than a text-model one, and silently
    /// corrupting the document is the worse failure.
    pub fn new(mut changes: Vec<Change>) -> Self {
        changes.sort_by_key(|change| (change.from, change.to));
        let mut ordered: Vec<Change> = Vec::with_capacity(changes.len());
        for change in changes {
            match ordered.last() {
                Some(previous) if change.from < previous.to => continue,
                Some(previous) if change.from == previous.to && change.from == previous.from => {
                    // Two pure insertions at the same point: keep both, in order.
                    ordered.push(change);
                }
                _ => ordered.push(change),
            }
        }
        Self { changes: ordered }
    }

    pub fn change(from: Offset, to: Offset, text: impl Into<String>) -> Self {
        Self::new(vec![Change::new(from, to, text)])
    }

    pub fn insert(at: Offset, text: impl Into<String>) -> Self {
        Self::change(at, at, text)
    }

    pub fn delete(from: Offset, to: Offset) -> Self {
        Self::change(from, to, "")
    }

    pub fn is_empty(&self) -> bool {
        self.changes
            .iter()
            .all(|change| change.from == change.to && change.text.is_empty())
    }

    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// Characters of replacement text this transaction carries. Undo history
    /// cost is the sum of this across retained entries, which is what makes it
    /// proportional to edit size rather than document size.
    pub fn footprint(&self) -> usize {
        self.changes
            .iter()
            .map(|change| change.text.chars().count())
            .sum()
    }

    /// Applies the transaction, returning its inverse.
    ///
    /// Forward changes are applied in descending order so that the offsets
    /// recorded against the original document stay valid as the rope shrinks
    /// and grows beneath them.
    pub fn apply(&self, rope: &mut Rope) -> Revert {
        let mut inverse = Vec::with_capacity(self.changes.len());
        let mut delta: isize = 0;
        for change in &self.changes {
            let removed = rope
                .get_slice(change.from..change.to)
                .map(|slice| slice.to_string())
                .unwrap_or_default();
            let new_from = (change.from as isize + delta) as usize;
            let new_to = new_from + change.inserted_len();
            inverse.push(Change::new(new_from, new_to, removed));
            delta += change.delta();
        }

        for change in self.changes.iter().rev() {
            if change.to > change.from {
                rope.remove(change.from..change.to);
            }
            if !change.text.is_empty() {
                rope.insert(change.from, &change.text);
            }
        }

        Revert { changes: inverse }
    }

    /// Maps an offset in the pre-transaction document to the post-transaction
    /// document.
    pub fn map_offset(&self, offset: Offset, assoc: Assoc) -> Offset {
        let mut delta: isize = 0;
        for change in &self.changes {
            if change.from == change.to && change.from == offset {
                // A pure insertion exactly at the offset: only an `After`
                // association moves past the inserted text.
                if assoc == Assoc::After {
                    delta += change.delta();
                }
                continue;
            }
            if change.to <= offset {
                delta += change.delta();
                continue;
            }
            if change.from >= offset {
                break;
            }
            // The offset pointed inside a replaced region.
            let start = (change.from as isize + delta) as usize;
            return match assoc {
                Assoc::Before => start,
                Assoc::After => start + change.inserted_len(),
            };
        }
        (offset as isize + delta).max(0) as usize
    }
}

/// Tickets handed out to every document that comes into being or changes.
///
/// The counter is global rather than per-document on purpose. A revision is
/// used to ask "is this still the text I last looked at", and a document that
/// was replaced wholesale — reloaded from disk, reset to a baseline — has to
/// answer no. Per-document counters would restart at zero and quietly say yes.
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

/// A rope-backed document.
#[derive(Clone, Debug)]
pub struct Text {
    rope: Rope,
    revision: u64,
}

impl Default for Text {
    fn default() -> Self {
        Self::new()
    }
}

impl Text {
    pub fn new() -> Self {
        Self {
            rope: Rope::new(),
            revision: next_revision(),
        }
    }

    /// Mirrors `Rope::from_str`, which this wraps. Deliberately not
    /// `std::str::FromStr`: construction is infallible, so an associated
    /// `Err` type would only add noise at every call site.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Self {
        Self {
            rope: Rope::from_str(value),
            revision: next_revision(),
        }
    }

    /// An opaque stamp that changes whenever this text does.
    ///
    /// Two texts with the same revision are the same text. Nothing else about
    /// the value is meaningful: it is not a count of edits, and it is not
    /// comparable in any order that means something.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }

    /// Whether this text has exactly the same characters as another document.
    ///
    /// A cloned [`Text`] shares Ropey's underlying chunks, so buffers can keep
    /// a saved baseline without copying the whole file. Comparing characters
    /// keeps the check correct after an edit produces the same content through
    /// a different rope shape.
    pub fn same_content(&self, other: &Self) -> bool {
        self.len_chars() == other.len_chars() && self.rope.chars().eq(other.rope.chars())
    }

    /// Number of rows. A document always has at least one row, and a trailing
    /// newline produces a final empty row, matching `split('\n')`.
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn last_row(&self) -> usize {
        self.len_lines().saturating_sub(1)
    }

    pub fn line(&self, row: usize) -> RopeSlice<'_> {
        self.rope.line(row.min(self.last_row()))
    }

    /// Character length of a row, excluding its line terminator.
    pub fn line_len(&self, row: usize) -> usize {
        if row >= self.len_lines() {
            return 0;
        }
        let line = self.rope.line(row);
        let mut len = line.len_chars();
        if len > 0 && line.char(len - 1) == '\n' {
            len -= 1;
            if len > 0 && line.char(len - 1) == '\r' {
                len -= 1;
            }
        }
        len
    }

    pub fn line_string(&self, row: usize) -> String {
        if row >= self.len_lines() {
            return String::new();
        }
        let len = self.line_len(row);
        let start = self.rope.line_to_char(row);
        self.slice_string(start, start + len)
    }

    pub fn line_to_offset(&self, row: usize) -> Offset {
        self.rope.line_to_char(row.min(self.last_row()))
    }

    pub fn offset_to_row(&self, offset: Offset) -> usize {
        self.rope.char_to_line(offset.min(self.len_chars()))
    }

    /// Converts a view coordinate to an offset, clamping both axes.
    pub fn offset_of(&self, position: Position) -> Offset {
        let row = position.row.min(self.last_row());
        let col = position.col.min(self.line_len(row));
        self.rope.line_to_char(row) + col
    }

    /// Converts an offset to a view coordinate.
    pub fn position_of(&self, offset: Offset) -> Position {
        let offset = offset.min(self.len_chars());
        let row = self.rope.char_to_line(offset);
        Position {
            row,
            col: offset - self.rope.line_to_char(row),
        }
    }

    pub fn char_at(&self, offset: Offset) -> Option<char> {
        if offset >= self.len_chars() {
            return None;
        }
        Some(self.rope.char(offset))
    }

    pub fn slice_string(&self, from: Offset, to: Offset) -> String {
        let len = self.len_chars();
        let from = from.min(len);
        let to = to.min(len);
        if from >= to {
            return String::new();
        }
        self.rope.slice(from..to).to_string()
    }

    /// Clamps an offset to the document.
    ///
    /// In Normal mode the caret sits *on* a character, so it may not rest on a
    /// row's terminator unless the row is empty. In Insert mode it may.
    pub fn clamp_offset(&self, offset: Offset, insert: bool) -> Offset {
        let offset = offset.min(self.len_chars());
        if insert {
            return offset;
        }
        let row = self.offset_to_row(offset);
        let start = self.line_to_offset(row);
        let len = self.line_len(row);
        let max = start + len.saturating_sub(1);
        if len == 0 { start } else { offset.min(max) }
    }

    pub fn apply(&mut self, transaction: &Transaction) -> Revert {
        self.revision = next_revision();
        transaction.apply(&mut self.rope)
    }

    pub fn lines(&self) -> impl Iterator<Item = String> + '_ {
        (0..self.len_lines()).map(|row| self.line_string(row))
    }

    /// The one change that turns `self` into `after`, or `None` if the two
    /// already hold the same text.
    ///
    /// Found by trimming the common prefix and suffix, which leaves the single
    /// contiguous span that differs. Applying the result to `self` reproduces
    /// `after` exactly, which is the whole contract: it is used where the
    /// individual transactions that produced the difference are no longer
    /// available, and their separate boundaries would tell a consumer nothing
    /// this does not.
    ///
    /// Both scans walk characters in order from one end, so the cost is the
    /// size of the matching run rather than a comparison of whole documents.
    pub fn change_to(&self, after: &Self) -> Option<Transaction> {
        let before_len = self.len_chars();
        let after_len = after.len_chars();
        let shortest = before_len.min(after_len);

        let mut prefix = 0;
        let mut before_head = self.rope.chars();
        let mut after_head = after.rope.chars();
        while prefix < shortest && before_head.next() == after_head.next() {
            prefix += 1;
        }
        if prefix == before_len && prefix == after_len {
            return None;
        }

        // Never past the prefix from the other end, so the two spans cannot
        // overlap and claim the same character twice.
        let mut suffix = 0;
        let limit = shortest - prefix;
        let mut before_tail = self.rope.chars_at(before_len);
        let mut after_tail = after.rope.chars_at(after_len);
        while suffix < limit && before_tail.prev() == after_tail.prev() {
            suffix += 1;
        }

        Some(Transaction::change(
            prefix,
            before_len - suffix,
            after.slice_string(prefix, after_len - suffix),
        ))
    }

    /// Bytes in the longest line, as an upper bound on its character count.
    ///
    /// Read straight from the rope's chunks instead of line by line: the
    /// answer only decides whether a document is cheap enough to soft-wrap,
    /// and a byte count can only overstate a character count, so a scan that
    /// never indexes a line or allocates one is enough to make that call.
    pub fn longest_line_bytes(&self) -> usize {
        let mut longest = 0;
        let mut current = 0;
        for chunk in self.rope.chunks() {
            let mut rest = chunk.as_bytes();
            while let Some(index) = rest.iter().position(|byte| *byte == b'\n') {
                longest = longest.max(current + index);
                current = 0;
                rest = &rest[index + 1..];
            }
            current += rest.len();
        }
        longest.max(current)
    }
}

impl fmt::Display for Text {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.rope)
    }
}

impl From<&str> for Text {
    fn from(value: &str) -> Self {
        Self::from_str(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_has_one_row() {
        let text = Text::new();
        assert_eq!(text.len_lines(), 1);
        assert_eq!(text.line_len(0), 0);
        assert_eq!(text.line_string(0), "");
    }

    #[test]
    fn trailing_newline_produces_a_final_empty_row() {
        let text = Text::from_str("a\nb\n");
        assert_eq!(text.len_lines(), 3);
        assert_eq!(text.line_string(2), "");
    }

    #[test]
    fn positions_round_trip_through_offsets() {
        let text = Text::from_str("aβc\n🦀d\n");
        for offset in 0..=text.len_chars() {
            let position = text.position_of(offset);
            assert_eq!(text.offset_of(position), offset, "offset {offset}");
        }
    }

    #[test]
    fn line_len_excludes_the_terminator() {
        let text = Text::from_str("abc\nde");
        assert_eq!(text.line_len(0), 3);
        assert_eq!(text.line_len(1), 2);
    }

    #[test]
    fn transactions_invert_exactly() {
        let mut text = Text::from_str("hello world");
        let transaction = Transaction::change(0, 5, "goodbye");
        let revert = text.apply(&transaction);
        assert_eq!(text.to_string(), "goodbye world");
        text.apply(&revert.into_transaction());
        assert_eq!(text.to_string(), "hello world");
    }

    #[test]
    fn multi_range_transactions_apply_and_invert() {
        let mut text = Text::from_str("a b c d");
        let transaction = Transaction::new(vec![
            Change::new(0, 1, "X"),
            Change::new(2, 3, "Y"),
            Change::new(6, 7, "Z"),
        ]);
        let revert = text.apply(&transaction);
        assert_eq!(text.to_string(), "X Y c Z");
        text.apply(&revert.into_transaction());
        assert_eq!(text.to_string(), "a b c d");
    }

    #[test]
    fn multi_range_transactions_handle_uneven_lengths() {
        let mut text = Text::from_str("aa bb cc");
        let transaction = Transaction::new(vec![
            Change::new(0, 2, "LONGER"),
            Change::new(3, 5, ""),
            Change::new(6, 8, "x"),
        ]);
        let revert = text.apply(&transaction);
        assert_eq!(text.to_string(), "LONGER  x");
        text.apply(&revert.into_transaction());
        assert_eq!(text.to_string(), "aa bb cc");
    }

    #[test]
    fn overlapping_changes_are_dropped_rather_than_corrupting() {
        let transaction =
            Transaction::new(vec![Change::new(0, 5, "abc"), Change::new(3, 8, "def")]);
        assert_eq!(transaction.changes().len(), 1);
    }

    #[test]
    fn offsets_map_through_insertions() {
        let transaction = Transaction::insert(3, "xx");
        assert_eq!(transaction.map_offset(0, Assoc::After), 0);
        assert_eq!(transaction.map_offset(4, Assoc::After), 6);
    }

    #[test]
    fn an_insertion_at_the_offset_moves_only_an_after_association() {
        let transaction = Transaction::insert(3, "xx");
        assert_eq!(transaction.map_offset(3, Assoc::After), 5);
        assert_eq!(transaction.map_offset(3, Assoc::Before), 3);
    }

    #[test]
    fn offsets_inside_a_deletion_collapse_by_association() {
        let transaction = Transaction::change(2, 6, "ab");
        assert_eq!(transaction.map_offset(4, Assoc::Before), 2);
        assert_eq!(transaction.map_offset(4, Assoc::After), 4);
        assert_eq!(transaction.map_offset(6, Assoc::After), 4);
    }

    #[test]
    fn clamping_respects_normal_and_insert_modes() {
        let text = Text::from_str("abc\n\ndef");
        // Row 0 spans offsets 0..3, terminator at 3.
        assert_eq!(text.clamp_offset(3, false), 2);
        assert_eq!(text.clamp_offset(3, true), 3);
        // Row 1 is empty; the caret may rest on its start.
        assert_eq!(text.clamp_offset(4, false), 4);
    }

    /// The contract is that the returned change reproduces `after` exactly,
    /// so every case is checked by applying it rather than by inspecting it.
    #[track_caller]
    fn assert_change_round_trips(before: &str, after: &str) {
        let start = Text::from_str(before);
        let end = Text::from_str(after);
        let mut result = start.clone();
        match start.change_to(&end) {
            Some(transaction) => {
                result.apply(&transaction);
            }
            None => assert_eq!(before, after, "no change reported for differing texts"),
        }
        assert_eq!(result.to_string(), after);
    }

    #[test]
    fn a_change_between_two_texts_reproduces_the_second() {
        assert_change_round_trips("", "");
        assert_change_round_trips("abc", "abc");
        assert_change_round_trips("", "inserted");
        assert_change_round_trips("removed", "");
        // Insertions at each end and in the middle.
        assert_change_round_trips("bc", "abc");
        assert_change_round_trips("ab", "abc");
        assert_change_round_trips("ac", "abc");
        // Deletions at each end and in the middle.
        assert_change_round_trips("abc", "bc");
        assert_change_round_trips("abc", "ab");
        assert_change_round_trips("abc", "ac");
        // Replacements, including ones that change length in both directions.
        assert_change_round_trips("alpha beta", "alpha gamma");
        assert_change_round_trips("alpha gamma", "alpha beta");
        assert_change_round_trips("one\ntwo\nthree", "one\ntwo two\nthree");
        assert_change_round_trips("one\ntwo\nthree", "one\nthree");
        // A repeated run: the prefix and suffix scans must not overlap and
        // consume the same characters twice.
        assert_change_round_trips("aaaa", "aaaaa");
        assert_change_round_trips("aaaaa", "aaaa");
        assert_change_round_trips("aaaa", "aa");
        // Multibyte, where character and byte offsets disagree.
        assert_change_round_trips("zażółć gęślą", "zażółć jaźń gęślą");
        assert_change_round_trips("zażółć gęślą", "zażółć");
        assert_change_round_trips("界界界", "界x界界");
    }

    #[test]
    fn identical_texts_report_no_change() {
        let text = Text::from_str("alpha\nbeta");
        assert!(text.change_to(&text.clone()).is_none());
        assert!(Text::new().change_to(&Text::new()).is_none());
    }

    #[test]
    fn a_change_between_two_texts_covers_only_what_differs() {
        let before = Text::from_str("keep this alpha and this");
        let after = Text::from_str("keep this beta and this");
        let transaction = before.change_to(&after).unwrap();
        let [change] = transaction.changes() else {
            panic!("expected exactly one change, got {transaction:?}");
        };
        // Minimal on both sides: "alpha" and "beta" share their final "a", so
        // the trailing scan claims it and the change stops short of it.
        assert_eq!((change.from, change.to), (10, 14));
        assert_eq!(change.text, "bet");
    }
}
