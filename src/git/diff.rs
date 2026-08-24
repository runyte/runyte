// SPDX-License-Identifier: MPL-2.0

//! Which rows of a buffer differ from the text Git has for it.
//!
//! The diff runs inside Runyte rather than in a subprocess, and that is the
//! point. Git is asked once for what a file looked like when it was staged;
//! every keystroke after that is compared against the text already in memory,
//! so a live gutter costs no processes and marks never lag behind the buffer
//! they describe.
//!
//! The comparison itself is not here. [`crate::diff`] owns the one line
//! alignment in Runyte, and this module turns it into the marks a gutter
//! draws: what happened to each row of the *current* text, with deletions
//! folded onto the row that closed over them because they have no row of
//! their own.
//!
//! Lines are compared whole and by content. Trailing newlines are not a line,
//! so adding or removing the final one is invisible here — Git reports it,
//! this gutter does not.

use crate::diff::{Run, Side};

/// What happened to one row of the current text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineChange {
    /// The row is new: nothing in the base corresponds to it.
    Added,
    /// The row replaces one that used to say something else.
    Modified,
    /// Rows were deleted immediately above this one.
    RemovedAbove,
    /// Rows were deleted after the last row, so the mark hangs off the end.
    RemovedBelow,
}

/// One marked row of the current text, by zero-based row index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RowChange {
    pub row: usize,
    pub change: LineChange,
}

/// What one line of a unified diff is, for a reader looking at it.
///
/// This is about presenting a patch, not about applying one: a frontend uses
/// it to choose a colour, and nothing else depends on it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffLine {
    Added,
    Removed,
    /// A `@@ -a,b +c,d @@` position line.
    Hunk,
    /// Anything that is not part of the patched text: the `diff --git` line,
    /// object ids, file modes, `\ No newline at end of file`, and any heading
    /// the editor itself put above the patch.
    Meta,
}

/// Reads one line of a unified diff in the context of its neighbours.
///
/// A line of patched text always begins with a space, `+`, or `-`, which is
/// what makes the classification total: anything beginning with something else
/// is not patched text and so is heading of one kind or another. `None` is a
/// context line, which is ordinary text and should look like it.
///
/// The neighbours settle the one genuine ambiguity. `--- a/file` and
/// `+++ b/file` are headings, but they are indistinguishable in isolation from
/// the removal of a line reading `-- a/file`, which is not hypothetical: any
/// Markdown or YAML file with a `---` in it produces exactly that. They are
/// always written as an adjacent pair, so the pair is what identifies them.
pub fn classify_line(line: &str, previous: Option<&str>, next: Option<&str>) -> Option<DiffLine> {
    let heading_pair = |first: Option<&str>, second: Option<&str>| {
        first.is_some_and(|line| line.starts_with("--- "))
            && second.is_some_and(|line| line.starts_with("+++ "))
    };
    match line.as_bytes().first()? {
        b' ' => None,
        b'+' if line.starts_with("+++ ") && heading_pair(previous, Some(line)) => {
            Some(DiffLine::Meta)
        }
        b'+' => Some(DiffLine::Added),
        b'-' if line.starts_with("--- ") && heading_pair(Some(line), next) => Some(DiffLine::Meta),
        b'-' => Some(DiffLine::Removed),
        b'@' if line.starts_with("@@") => Some(DiffLine::Hunk),
        _ => Some(DiffLine::Meta),
    }
}

/// The rows of `current` that differ from `base`, in ascending row order.
///
/// This is one reading of the shared line alignment in [`crate::diff`]: the
/// marks a gutter draws beside the current text. Because a side-by-side view
/// reads the same alignment, the two surfaces cannot disagree about what
/// changed.
pub fn changed_rows(base: &str, current: &str) -> Vec<RowChange> {
    let alignment = crate::diff::align_text(base, current);
    let current_lines = alignment.lines(Side::Right);
    let mut rows = Vec::new();
    for run in alignment.changed() {
        emit(&mut rows, current_lines, run);
    }
    rows
}

fn emit(rows: &mut Vec<RowChange>, current_lines: usize, run: &Run) {
    let at = run.right.start;
    if !run.right.is_empty() {
        // Lines that replace other lines are modified rather than added. When
        // the two runs are unequal the surplus is folded into the same marks:
        // a second marker for the leftover deletions would point at rows that
        // are already flagged.
        let change = if run.left.is_empty() {
            LineChange::Added
        } else {
            LineChange::Modified
        };
        rows.extend(run.right.clone().map(|row| RowChange { row, change }));
    } else if current_lines > 0 {
        // A pure deletion has no row of its own, so it marks the row that
        // closed over the gap — or the last row, when the gap is at the end.
        rows.push(if at < current_lines {
            RowChange {
                row: at,
                change: LineChange::RemovedAbove,
            }
        } else {
            RowChange {
                row: current_lines - 1,
                change: LineChange::RemovedBelow,
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(base: &str, current: &str) -> Vec<(usize, LineChange)> {
        changed_rows(base, current)
            .into_iter()
            .map(|change| (change.row, change.change))
            .collect()
    }

    #[test]
    fn identical_text_has_no_changed_rows() {
        assert!(rows("a\nb\nc\n", "a\nb\nc\n").is_empty());
        assert!(rows("", "").is_empty());
    }

    /// The final newline is not a line, so writing one is not a change.
    #[test]
    fn a_trailing_newline_is_not_a_changed_row() {
        assert!(rows("a\nb", "a\nb\n").is_empty());
    }

    #[test]
    fn an_edited_line_is_modified_and_its_neighbours_are_untouched() {
        assert_eq!(
            rows("a\nb\nc\n", "a\nB\nc\n"),
            vec![(1, LineChange::Modified)]
        );
    }

    #[test]
    fn inserted_lines_are_added() {
        assert_eq!(
            rows("a\nc\n", "a\nb1\nb2\nc\n"),
            vec![(1, LineChange::Added), (2, LineChange::Added)]
        );
    }

    #[test]
    fn a_new_file_against_an_empty_base_is_all_added() {
        assert_eq!(
            rows("", "a\nb\n"),
            vec![(0, LineChange::Added), (1, LineChange::Added)]
        );
    }

    /// A deletion has no row, so it marks the row that closed over it.
    #[test]
    fn a_deletion_marks_the_row_below_the_gap() {
        assert_eq!(
            rows("a\nb\nc\n", "a\nc\n"),
            vec![(1, LineChange::RemovedAbove)]
        );
    }

    #[test]
    fn a_deletion_at_the_end_hangs_off_the_last_row() {
        assert_eq!(
            rows("a\nb\nc\n", "a\n"),
            vec![(0, LineChange::RemovedBelow)]
        );
    }

    #[test]
    fn deleting_everything_leaves_no_row_to_mark() {
        assert!(rows("a\nb\n", "").is_empty());
    }

    /// Replacing two lines with three is one change, not a deletion marker
    /// stacked on top of three additions.
    #[test]
    fn an_uneven_replacement_marks_only_the_new_rows() {
        assert_eq!(
            rows("a\nx\ny\nb\n", "a\np\nq\nr\nb\n"),
            vec![
                (1, LineChange::Modified),
                (2, LineChange::Modified),
                (3, LineChange::Modified),
            ]
        );
    }

    #[test]
    fn several_regions_are_reported_independently() {
        assert_eq!(
            rows("a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\nf\n"),
            vec![(1, LineChange::Modified), (5, LineChange::Added)]
        );
    }

    #[test]
    fn repeated_lines_align_on_the_nearest_match() {
        // A gutter that mistook which `}` moved would light up the whole
        // block instead of the one inserted line.
        assert_eq!(
            rows("if a {\n}\nif b {\n}\n", "if a {\n    x\n}\nif b {\n}\n"),
            vec![(1, LineChange::Added)]
        );
    }

    /// Beyond the alignment bound the region is still reported, coarsely.
    #[test]
    fn an_enormous_region_is_reported_without_aligning_it() {
        let base = (0..2000)
            .map(|line| format!("base {line}\n"))
            .collect::<String>();
        let current = (0..2000)
            .map(|line| format!("current {line}\n"))
            .collect::<String>();

        let rows = changed_rows(&base, &current);
        assert_eq!(rows.len(), 2000);
        assert!(rows.iter().all(|row| row.change == LineChange::Modified));
        assert_eq!(rows[0].row, 0);
        assert_eq!(rows[1999].row, 1999);
    }

    /// Only the region between the shared ends is aligned, so a small edit in
    /// a very large file stays cheap and exact.
    #[test]
    fn shared_ends_keep_a_large_file_exact() {
        let mut base = (0..50_000)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        let mut current = base.clone();
        base.push_str("tail\n");
        current.push_str("TAIL\n");

        assert_eq!(
            changed_rows(&base, &current),
            vec![RowChange {
                row: 50_000,
                change: LineChange::Modified
            }]
        );
    }

    /// Every line of a real patch, read the way a reader sees it.
    #[test]
    fn a_patch_reads_as_headings_positions_and_changed_text() {
        let patch = [
            "# not staged · lorem.md",
            "",
            "diff --git a/lorem.md b/lorem.md",
            "index f5296cc..d3bfa3b 100644",
            "--- a/lorem.md",
            "+++ b/lorem.md",
            "@@ -15,11 +15,10 @@ context after the position",
            " unchanged",
            "-removed",
            "+added",
            "\\ No newline at end of file",
        ];

        assert_eq!(
            classify(&patch),
            vec![
                Some(DiffLine::Meta),
                None,
                Some(DiffLine::Meta),
                Some(DiffLine::Meta),
                Some(DiffLine::Meta),
                Some(DiffLine::Meta),
                Some(DiffLine::Hunk),
                None,
                Some(DiffLine::Removed),
                Some(DiffLine::Added),
                Some(DiffLine::Meta),
            ]
        );
    }

    /// Removing a line that itself starts with `--` produces a diff line
    /// starting with `---`, which is not a heading. Markdown and YAML are full
    /// of them.
    #[test]
    fn a_removed_line_starting_with_dashes_is_not_a_heading() {
        let patch = [
            "@@ -1,3 +1,3 @@",
            "--- a rule in the text",
            "+++ replaced with this",
            " context",
        ];

        assert_eq!(
            classify(&patch),
            vec![
                Some(DiffLine::Hunk),
                // Adjacent, so the pair rule alone would call these headings;
                // a heading pair never follows a hunk position.
                Some(DiffLine::Meta),
                Some(DiffLine::Meta),
                None,
            ]
        );
        // The unpaired case, which is the common one, is read correctly.
        assert_eq!(
            classify(&["@@ -1 +1 @@", "--- One", "+- One abc"]),
            vec![
                Some(DiffLine::Hunk),
                Some(DiffLine::Removed),
                Some(DiffLine::Added),
            ]
        );
    }

    #[test]
    fn context_lines_and_blank_lines_are_ordinary_text() {
        assert_eq!(classify_line(" unchanged", None, None), None);
        assert_eq!(classify_line("", None, None), None);
    }

    /// Anything that is not patched text is heading, including text the editor
    /// itself put above the patch.
    #[test]
    fn unrecognised_lines_are_headings_rather_than_text() {
        for line in [
            "new file mode 100644",
            "deleted file mode 100644",
            "similarity index 95%",
            "rename from old.rs",
            "Binary files a/x.png and b/x.png differ",
            "# staged for commit · 2 files",
        ] {
            assert_eq!(
                classify_line(line, None, None),
                Some(DiffLine::Meta),
                "{line}"
            );
        }
    }

    fn classify(lines: &[&str]) -> Vec<Option<DiffLine>> {
        (0..lines.len())
            .map(|index| {
                classify_line(
                    lines[index],
                    index.checked_sub(1).map(|previous| lines[previous]),
                    lines.get(index + 1).copied(),
                )
            })
            .collect()
    }
}
