// SPDX-License-Identifier: MPL-2.0

//! Two buffers shown side by side, and what ties them together.
//!
//! A session is deliberately thin. It names two panes and the two buffers they
//! show, caches the [`Alignment`] between those buffers' text, and may remember
//! the buffer a temporary paired presentation replaced. It does not know what
//! a file is, so anything that can produce a buffer can be a side: the
//! file-against-file view this was written for, and a Git base against a
//! working tree later, are the same object with different buffers in it.
//!
//! Nothing here draws or scrolls. The session answers two questions — how does
//! this row of this side read, and which row of the other side sits level with
//! it — and the editor's existing pane projection does the rest.

use crate::diff::{Alignment, Change, Side, align_text};

/// The largest text that will be aligned.
///
/// Past this size a side-by-side view stops being something a person reads and
/// starts being something that stalls a keystroke. Such a pair is refused when
/// the view is opened rather than opened and left wrong.
pub const MAX_DIFF_BYTES: usize = 4 * 1024 * 1024;

/// One side of a live diff: the pane showing it, and the buffer it shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffSide {
    pub pane: usize,
    pub buffer: usize,
}

/// Two buffers shown side by side.
#[derive(Clone, Debug)]
pub struct DiffSession {
    left: DiffSide,
    right: DiffSide,
    /// Buffer a temporary paired view replaced, if closing either pane should
    /// collapse the pair and return the survivor to that buffer.
    ///
    /// Ordinary `:diff-this` sessions borrow whatever panes already show
    /// their buffers and leave this absent. Git's complete-version view
    /// creates both sides as one temporary presentation over the active pane,
    /// so either close dismisses the whole presentation instead.
    pane_close_return: Option<usize>,
    alignment: Alignment,
    /// The buffer revisions `alignment` was computed from, left then right.
    /// Both sides stay editable, so the alignment is only ever as current as
    /// the revisions it was built from.
    revisions: (u64, u64),
    /// Where both viewports start in the aligned row space.
    ///
    /// This single value is the whole of the scroll link. It is derived each
    /// frame from whichever pane is leading, and both panes then project the
    /// same stretch of aligned rows, so neither has to know where the other
    /// one is.
    aligned_start: usize,
}

impl DiffSession {
    pub fn new(left: DiffSide, right: DiffSide, left_text: &str, right_text: &str) -> Self {
        Self {
            left,
            right,
            pane_close_return: None,
            alignment: align_text(left_text, right_text),
            // A fresh session has not seen a revision yet, so the first update
            // always recomputes rather than trusting a default.
            revisions: (u64::MAX, u64::MAX),
            aligned_start: 0,
        }
    }

    pub fn aligned_start(&self) -> usize {
        self.aligned_start
    }

    pub fn set_aligned_start(&mut self, aligned: usize) {
        self.aligned_start = aligned;
    }

    pub fn side(&self, side: Side) -> DiffSide {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
        }
    }

    pub fn alignment(&self) -> &Alignment {
        &self.alignment
    }

    /// Which side a pane is, if this session owns it.
    pub fn side_of_pane(&self, pane: usize) -> Option<Side> {
        if self.left.pane == pane {
            Some(Side::Left)
        } else if self.right.pane == pane {
            Some(Side::Right)
        } else {
            None
        }
    }

    pub fn has_pane(&self, pane: usize) -> bool {
        self.side_of_pane(pane).is_some()
    }

    pub fn has_buffer(&self, buffer: usize) -> bool {
        self.left.buffer == buffer || self.right.buffer == buffer
    }

    pub fn panes(&self) -> [usize; 2] {
        [self.left.pane, self.right.pane]
    }

    /// Makes the two panes one temporary presentation for close purposes.
    pub(crate) fn returning_on_pane_close(mut self, buffer: usize) -> Self {
        self.pane_close_return = Some(buffer);
        self
    }

    pub(crate) fn pane_close_return(&self) -> Option<usize> {
        self.pane_close_return
    }

    /// Moves a comparison's pane ownership with the pane contents being
    /// exchanged. The side remains attached to its buffer, even when that
    /// buffer lands on the other side of the split.
    pub(crate) fn swap_panes(&mut self, first: usize, second: usize) {
        for side in [&mut self.left, &mut self.right] {
            if side.pane == first {
                side.pane = second;
            } else if side.pane == second {
                side.pane = first;
            }
        }
    }

    /// Whether either side's text has moved on since the alignment was built.
    ///
    /// Asked separately from [`Self::update`] so a caller does not have to
    /// materialise two buffers' text on the frames where nothing changed,
    /// which is nearly all of them.
    pub fn needs_update(&self, revisions: (u64, u64)) -> bool {
        self.revisions != revisions
    }

    /// Rebuilds the alignment from text the caller has already fetched.
    pub fn update(&mut self, revisions: (u64, u64), left: &str, right: &str) {
        self.alignment = align_text(left, right);
        self.revisions = revisions;
    }

    /// How one row of one side reads, or `None` where it matches the other.
    pub fn change(&self, side: Side, row: usize) -> Option<Change> {
        self.alignment.change(side, row)
    }

    /// The row of `side` that sits level with `row` of the other side.
    ///
    /// A row facing filler has no counterpart, so the answer is the nearest
    /// real row at or above it: a follower pane scrolled to a filler would
    /// otherwise have nowhere to be.
    pub fn facing_row(&self, from: Side, row: usize) -> usize {
        let aligned = self.alignment.aligned_row(from, row);
        self.row_at_or_above(from.opposite(), aligned)
    }

    /// The nearest row of `side` at or above an aligned row.
    pub fn row_at_or_above(&self, side: Side, aligned: usize) -> usize {
        for candidate in (0..=aligned).rev() {
            if let Some(row) = self.alignment.row_at(side, candidate) {
                return row;
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(left: &str, right: &str) -> DiffSession {
        DiffSession::new(
            DiffSide { pane: 0, buffer: 0 },
            DiffSide { pane: 1, buffer: 1 },
            left,
            right,
        )
    }

    #[test]
    fn a_session_knows_which_side_a_pane_is() {
        let session = session("a\n", "b\n");
        assert_eq!(session.side_of_pane(0), Some(Side::Left));
        assert_eq!(session.side_of_pane(1), Some(Side::Right));
        assert_eq!(session.side_of_pane(2), None);
        assert!(session.has_buffer(1));
        assert!(!session.has_buffer(9));
    }

    /// Equal lines sit level, which is the whole point of the aligned space.
    #[test]
    fn equal_lines_face_each_other() {
        let session = session("a\nb\nc\n", "a\nb\nc\n");
        for row in 0..3 {
            assert_eq!(session.facing_row(Side::Left, row), row);
            assert_eq!(session.facing_row(Side::Right, row), row);
        }
    }

    /// An insertion pushes the lines below it down on the right only, so the
    /// two sides face each other across the gap rather than by row number.
    #[test]
    fn an_insertion_offsets_the_rows_below_it() {
        let session = session("a\nc\n", "a\nb\nc\n");
        assert_eq!(session.facing_row(Side::Left, 1), 2);
        assert_eq!(session.facing_row(Side::Right, 2), 1);
    }

    /// A row facing filler falls back to the nearest real row above it, so a
    /// follower always has somewhere to sit.
    #[test]
    fn a_row_facing_filler_falls_back_to_the_row_above() {
        let session = session("a\nc\n", "a\nb\nc\n");
        // Right row 1 is the inserted line; the left side shows filler there,
        // so it falls back to the last real line above the gap.
        assert_eq!(session.facing_row(Side::Right, 1), 0);
        assert_eq!(session.row_at_or_above(Side::Left, 1), 0);
    }

    #[test]
    fn an_alignment_is_recomputed_only_when_a_revision_moves() {
        let mut session = session("a\n", "a\n");
        assert!(session.needs_update((1, 1)));
        session.update((1, 1), "a\nb\n", "a\n");
        assert_eq!(session.change(Side::Left, 1), Some(Change::Removed));
        assert!(!session.needs_update((1, 1)));
        assert!(session.needs_update((1, 2)));
    }

    #[test]
    fn swapping_pane_ownership_keeps_each_side_attached_to_its_buffer() {
        let mut session = session("left\n", "right\n");
        session.swap_panes(0, 1);

        assert_eq!(session.side(Side::Left), DiffSide { pane: 1, buffer: 0 });
        assert_eq!(session.side(Side::Right), DiffSide { pane: 0, buffer: 1 });
    }
}
