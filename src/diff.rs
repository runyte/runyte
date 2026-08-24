// SPDX-License-Identifier: MPL-2.0

//! How the lines of two texts correspond to each other.
//!
//! This is the one line-diff in Runyte. It knows nothing about Git, about
//! buffers, or about panes: it takes two slices of lines and says which runs
//! of them answer to which. Everything that shows a difference — the Git
//! gutter's per-row marks, a side-by-side view's alignment — is a reading of
//! the same [`Alignment`], so two surfaces can never disagree about what
//! changed.
//!
//! Lines are compared whole and by content. A trailing newline is not a line,
//! so adding or removing the final one is invisible here.
//!
//! Besides the runs themselves, an alignment defines an **aligned row space**:
//! the rows of a view in which corresponding lines sit at the same height.
//! A run occupies as many aligned rows as its longer side has lines, and the
//! shorter side leaves the remainder empty. That empty remainder is what a
//! frontend draws as filler, and it is what makes two views scroll together
//! without either of them knowing about the other.

use std::ops::Range;

/// How large a changed region may be before it is described rather than
/// aligned.
///
/// The comparison below is quadratic in the size of the region that is left
/// after identical text at both ends is trimmed away, which for editing is
/// almost always a handful of lines. A region that really is enormous — a
/// generated file replaced wholesale, a buffer pasted over — is reported as
/// changed without working out the correspondence line by line, because no
/// reader was going to study that mapping anyway.
const MAX_REGION_CELLS: usize = 1 << 20;

/// Which of the two texts a row belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// The other text, which is what a filler row is standing in for.
    pub const fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// What one run of lines is, read from the left text towards the right one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunKind {
    /// Both sides say the same thing.
    Equal,
    /// Both sides have lines here and they differ.
    Replaced,
    /// Only the right text has lines here.
    Inserted,
    /// Only the left text has lines here.
    Deleted,
}

/// One run of lines that answer to each other, by zero-based row.
///
/// Exactly one of the two ranges is empty for [`RunKind::Inserted`] and
/// [`RunKind::Deleted`]; neither is for the other kinds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Run {
    pub kind: RunKind,
    pub left: Range<usize>,
    pub right: Range<usize>,
    /// Where this run starts in the aligned row space.
    pub aligned: usize,
}

impl Run {
    /// The rows one side contributes to this run.
    pub fn side(&self, side: Side) -> Range<usize> {
        match side {
            Side::Left => self.left.clone(),
            Side::Right => self.right.clone(),
        }
    }

    /// How many aligned rows this run occupies.
    pub fn height(&self) -> usize {
        self.left.len().max(self.right.len())
    }

    /// How many filler rows one side needs to reach the run's full height.
    pub fn filler(&self, side: Side) -> usize {
        self.height() - self.side(side).len()
    }
}

/// What happened to one row of one side, for a reader looking at it.
///
/// This is a presentation reading of a [`Run`], not a second model of the
/// difference: a frontend uses it to choose a colour and nothing else depends
/// on it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Change {
    /// The right text has this line and the left one does not.
    Added,
    /// The left text has this line and the right one does not.
    Removed,
    /// The line answers to a different line on the other side.
    Changed,
}

/// The complete line correspondence between two texts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Alignment {
    runs: Vec<Run>,
    height: usize,
    left_lines: usize,
    right_lines: usize,
}

impl Alignment {
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// Rows in the aligned row space, filler included.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Lines one side actually has.
    pub fn lines(&self, side: Side) -> usize {
        match side {
            Side::Left => self.left_lines,
            Side::Right => self.right_lines,
        }
    }

    /// Whether the two texts say the same thing line for line.
    pub fn is_equal(&self) -> bool {
        self.runs.iter().all(|run| run.kind == RunKind::Equal)
    }

    /// The runs that are not [`RunKind::Equal`], in ascending row order.
    pub fn changed(&self) -> impl Iterator<Item = &Run> {
        self.runs.iter().filter(|run| run.kind != RunKind::Equal)
    }

    /// The run one side's row belongs to.
    ///
    /// A run that contributes no rows to `side` can never be returned, which
    /// is what makes the answer unambiguous where an empty range sits between
    /// two others that share its position.
    pub fn run_at(&self, side: Side, row: usize) -> Option<&Run> {
        // Ranges on one side are contiguous and ascending, so the last run
        // that starts at or before `row` is the only one that can contain it.
        // Taking the last rather than the first is what steps over the
        // zero-width runs the other side owns.
        let index = self
            .runs
            .partition_point(|run| run.side(side).start <= row)
            .checked_sub(1)?;
        let run = &self.runs[index];
        (row < run.side(side).end).then_some(run)
    }

    /// How to read one side's row, or `None` where it is unchanged.
    pub fn change(&self, side: Side, row: usize) -> Option<Change> {
        match (self.run_at(side, row)?.kind, side) {
            (RunKind::Equal, _) => None,
            (RunKind::Replaced, _) => Some(Change::Changed),
            (RunKind::Inserted, Side::Right) => Some(Change::Added),
            (RunKind::Deleted, Side::Left) => Some(Change::Removed),
            // A run with no rows on this side is never returned by `run_at`.
            (RunKind::Inserted, Side::Left) | (RunKind::Deleted, Side::Right) => None,
        }
    }

    /// Where one side's row sits in the aligned row space.
    ///
    /// Rows past the end of that side keep counting upwards from the bottom of
    /// the alignment, so a caller need not special-case a caret sitting on the
    /// last line of the shorter text.
    pub fn aligned_row(&self, side: Side, row: usize) -> usize {
        match self.run_at(side, row) {
            Some(run) => run.aligned + (row - run.side(side).start),
            None => self.height + row.saturating_sub(self.lines(side)),
        }
    }

    /// The row one side shows at an aligned row, or `None` for a filler.
    pub fn row_at(&self, side: Side, aligned: usize) -> Option<usize> {
        if aligned >= self.height {
            // Past the alignment both sides have run out together, so the
            // overflow is the same on each and the row is a real one.
            return Some(self.lines(side) + (aligned - self.height));
        }
        let index = self
            .runs
            .partition_point(|run| run.aligned <= aligned)
            .checked_sub(1)?;
        let run = &self.runs[index];
        let offset = aligned - run.aligned;
        let range = run.side(side);
        (offset < range.len()).then_some(range.start + offset)
    }
}

/// The line correspondence between two texts.
pub fn align_text(left: &str, right: &str) -> Alignment {
    let left = left.lines().collect::<Vec<_>>();
    let right = right.lines().collect::<Vec<_>>();
    align(&left, &right)
}

/// The line correspondence between two slices of lines.
pub fn align(left: &[&str], right: &[&str]) -> Alignment {
    let prefix = left
        .iter()
        .zip(right)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = left[prefix..]
        .iter()
        .rev()
        .zip(right[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let left_region = &left[prefix..left.len() - suffix];
    let right_region = &right[prefix..right.len() - suffix];

    let mut runs: Vec<Run> = Vec::new();
    let mut aligned = 0;
    let (mut left_at, mut right_at) = (0, 0);
    let push = |runs: &mut Vec<Run>,
                aligned: &mut usize,
                kind: RunKind,
                left: Range<usize>,
                right: Range<usize>| {
        if left.is_empty() && right.is_empty() {
            return;
        }
        let run = Run {
            kind,
            left,
            right,
            aligned: *aligned,
        };
        *aligned += run.height();
        runs.push(run);
    };

    for group in groups(left_region, right_region) {
        let left_start = prefix + group.left_at;
        let right_start = prefix + group.right_at;
        // Whatever the walk passed over between groups matched on both sides,
        // so the two gaps are the same length by construction.
        push(
            &mut runs,
            &mut aligned,
            RunKind::Equal,
            left_at..left_start,
            right_at..right_start,
        );
        let left_end = left_start + group.deleted;
        let right_end = right_start + group.inserted;
        let kind = match (group.deleted, group.inserted) {
            (0, _) => RunKind::Inserted,
            (_, 0) => RunKind::Deleted,
            _ => RunKind::Replaced,
        };
        push(
            &mut runs,
            &mut aligned,
            kind,
            left_start..left_end,
            right_start..right_end,
        );
        (left_at, right_at) = (left_end, right_end);
    }
    // The trailing run is the common suffix plus anything the last group left
    // behind, which is again equal on both sides.
    push(
        &mut runs,
        &mut aligned,
        RunKind::Equal,
        left_at..left.len(),
        right_at..right.len(),
    );

    Alignment {
        runs,
        height: aligned,
        left_lines: left.len(),
        right_lines: right.len(),
    }
}

/// A run of deleted left lines and inserted right lines at one place, by row
/// within the region that is left after the common ends are trimmed away.
#[derive(Clone, Copy, Debug)]
struct Group {
    /// Row in the left region where the deleted lines start. The two indices
    /// drift apart once a group has consumed different numbers of lines from
    /// each side, so neither can be derived from the other.
    left_at: usize,
    /// Row in the right region where the inserted lines start.
    right_at: usize,
    deleted: usize,
    inserted: usize,
}

fn groups(left: &[&str], right: &[&str]) -> Vec<Group> {
    if left.is_empty() && right.is_empty() {
        return Vec::new();
    }
    let whole = || {
        vec![Group {
            left_at: 0,
            right_at: 0,
            deleted: left.len(),
            inserted: right.len(),
        }]
    };
    if left.is_empty() || right.is_empty() {
        return whole();
    }
    match left.len().checked_mul(right.len()) {
        Some(cells) if cells <= MAX_REGION_CELLS => aligned_groups(left, right),
        // Too large to align: the whole region changed, which is true.
        _ => whole(),
    }
}

/// Groups derived from a longest-common-subsequence alignment.
fn aligned_groups(left: &[&str], right: &[&str]) -> Vec<Group> {
    let width = right.len() + 1;
    // `lengths[i * width + j]` is the length of the longest common
    // subsequence of `left[i..]` and `right[j..]`, filled from the end so
    // the forward walk below can follow the greedy choice at each step.
    let mut lengths = vec![0u32; (left.len() + 1) * width];
    for i in (0..left.len()).rev() {
        for j in (0..right.len()).rev() {
            lengths[i * width + j] = if left[i] == right[j] {
                lengths[(i + 1) * width + j + 1] + 1
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }

    let mut groups: Vec<Group> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < left.len() || j < right.len() {
        if i < left.len() && j < right.len() && left[i] == right[j] {
            i += 1;
            j += 1;
            continue;
        }
        let deletes = j == right.len()
            || i < left.len() && lengths[(i + 1) * width + j] >= lengths[i * width + j + 1];
        let group = match groups.last_mut() {
            // A deletion and the insertion that follows it are one change, so
            // a group continues while neither side has advanced past it.
            Some(group)
                if group.right_at + group.inserted == j && group.left_at + group.deleted == i =>
            {
                group
            }
            _ => {
                groups.push(Group {
                    left_at: i,
                    right_at: j,
                    deleted: 0,
                    inserted: 0,
                });
                groups.last_mut().expect("just pushed")
            }
        };
        if deletes {
            group.deleted += 1;
            i += 1;
        } else {
            group.inserted += 1;
            j += 1;
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(left: &str, right: &str) -> Vec<(RunKind, Range<usize>, Range<usize>)> {
        align_text(left, right)
            .runs
            .into_iter()
            .map(|run| (run.kind, run.left, run.right))
            .collect()
    }

    #[test]
    fn identical_text_is_one_equal_run() {
        assert_eq!(
            runs("a\nb\nc\n", "a\nb\nc\n"),
            [(RunKind::Equal, 0..3, 0..3)]
        );
        assert!(align_text("", "").runs.is_empty());
        assert!(align_text("a\nb\n", "a\nb\n").is_equal());
    }

    /// The final newline is not a line, so writing one changes nothing.
    #[test]
    fn a_trailing_newline_is_not_a_line() {
        assert!(align_text("a\nb", "a\nb\n").is_equal());
    }

    #[test]
    fn an_edited_line_replaces_the_one_it_answers_to() {
        assert_eq!(
            runs("a\nb\nc\n", "a\nB\nc\n"),
            [
                (RunKind::Equal, 0..1, 0..1),
                (RunKind::Replaced, 1..2, 1..2),
                (RunKind::Equal, 2..3, 2..3),
            ]
        );
    }

    #[test]
    fn lines_only_the_right_text_has_are_inserted() {
        assert_eq!(
            runs("a\nc\n", "a\nb1\nb2\nc\n"),
            [
                (RunKind::Equal, 0..1, 0..1),
                (RunKind::Inserted, 1..1, 1..3),
                (RunKind::Equal, 1..2, 3..4),
            ]
        );
    }

    #[test]
    fn lines_only_the_left_text_has_are_deleted() {
        assert_eq!(
            runs("a\nb\nc\n", "a\nc\n"),
            [
                (RunKind::Equal, 0..1, 0..1),
                (RunKind::Deleted, 1..2, 1..1),
                (RunKind::Equal, 2..3, 1..2),
            ]
        );
    }

    #[test]
    fn an_empty_side_is_one_run_over_the_whole_other_side() {
        assert_eq!(runs("", "a\nb\n"), [(RunKind::Inserted, 0..0, 0..2)]);
        assert_eq!(runs("a\nb\n", ""), [(RunKind::Deleted, 0..2, 0..0)]);
    }

    /// A run is as tall as its longer side, and the shorter side is padded.
    #[test]
    fn the_aligned_row_space_pads_the_shorter_side_of_a_run() {
        let alignment = align_text("a\nb\nc\nz\n", "a\nB1\nB2\nB3\nz\n");
        assert_eq!(alignment.height(), 5);
        let replaced = alignment
            .changed()
            .next()
            .expect("the middle lines differ")
            .clone();
        assert_eq!(replaced.kind, RunKind::Replaced);
        assert_eq!(replaced.height(), 3);
        assert_eq!(replaced.filler(Side::Left), 1);
        assert_eq!(replaced.filler(Side::Right), 0);

        // Left row 3 and right row 4 are the same line, so they align.
        assert_eq!(alignment.aligned_row(Side::Left, 3), 4);
        assert_eq!(alignment.aligned_row(Side::Right, 4), 4);
        // The left side has nothing to show against the third replacement.
        assert_eq!(alignment.row_at(Side::Left, 3), None);
        assert_eq!(alignment.row_at(Side::Right, 3), Some(3));
    }

    #[test]
    fn every_row_of_a_side_maps_back_to_itself() {
        let alignment = align_text("a\nb\nc\nd\n", "a\nX\nc\nd\ne\nf\n");
        for side in [Side::Left, Side::Right] {
            for row in 0..alignment.lines(side) {
                let aligned = alignment.aligned_row(side, row);
                assert_eq!(alignment.row_at(side, aligned), Some(row), "{side:?} {row}");
            }
        }
    }

    #[test]
    fn each_side_reads_its_own_rows_of_a_change() {
        let alignment = align_text("a\nb\nc\nd\ne\n", "a\nB\nc\ne\n");
        assert_eq!(alignment.change(Side::Left, 0), None);
        assert_eq!(alignment.change(Side::Left, 1), Some(Change::Changed));
        assert_eq!(alignment.change(Side::Right, 1), Some(Change::Changed));
        assert_eq!(alignment.change(Side::Left, 3), Some(Change::Removed));
        assert_eq!(alignment.change(Side::Right, 3), None);
    }

    /// Deletions that sit against a replacement are part of it. Reporting the
    /// surplus separately would claim two changes where a reader sees one, and
    /// it is the same folding the Git gutter does with its marks.
    #[test]
    fn surplus_lines_of_an_uneven_replacement_stay_part_of_it() {
        let alignment = align_text("a\nb\nc\n", "a\nB\n");
        assert_eq!(alignment.changed().count(), 1);
        assert_eq!(alignment.change(Side::Left, 1), Some(Change::Changed));
        assert_eq!(alignment.change(Side::Left, 2), Some(Change::Changed));
        assert_eq!(alignment.change(Side::Right, 1), Some(Change::Changed));
    }

    /// A zero-width run belongs to one side only, and must not swallow the row
    /// the other side has at the same position.
    #[test]
    fn an_empty_range_never_claims_the_other_sides_row() {
        let alignment = align_text("a\nc\n", "a\nb\nc\n");
        assert_eq!(alignment.change(Side::Left, 1), None);
        assert_eq!(alignment.change(Side::Right, 1), Some(Change::Added));
        assert_eq!(
            alignment.run_at(Side::Left, 1).map(|run| run.kind),
            Some(RunKind::Equal)
        );
    }

    /// Rows past the end of a side keep counting, so a caret on the last line
    /// of the shorter text still has an aligned position.
    #[test]
    fn rows_past_the_end_of_a_side_keep_counting() {
        let alignment = align_text("a\n", "a\n");
        assert_eq!(alignment.aligned_row(Side::Left, 5), 5);
        assert_eq!(alignment.row_at(Side::Left, 5), Some(5));
    }

    /// A region too large to align is reported as changed rather than mapped
    /// line by line, which is true and bounded.
    #[test]
    fn an_enormous_region_is_one_run() {
        let left = (0..1500).map(|i| format!("l{i}")).collect::<Vec<_>>();
        let right = (0..1500).map(|i| format!("r{i}")).collect::<Vec<_>>();
        let left = left.iter().map(String::as_str).collect::<Vec<_>>();
        let right = right.iter().map(String::as_str).collect::<Vec<_>>();
        let alignment = align(&left, &right);
        assert_eq!(alignment.runs().len(), 1);
        assert_eq!(alignment.runs()[0].kind, RunKind::Replaced);
        assert_eq!(alignment.height(), 1500);
    }
}
