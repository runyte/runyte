// SPDX-License-Identifier: MPL-2.0

//! The changed-file list, as a projection of repository status.
//!
//! Deterministic and presentation-neutral: the same status produces the same
//! rows, no state is kept, and nothing here decides a colour or a width. What
//! makes it a projection rather than a formatter is the second half of each
//! row — the file that a key pressed on that row acts on. Without it the list
//! would be a picture of the repository instead of a way to work on it.

use std::{ops::Range, path::PathBuf};

use super::{
    DiffScope, Divergence, FileState, FileStatus, LineStats, RepositoryStatus, StatusStats,
};

/// Which of a row's two counts a character stands in.
///
/// Its own kind rather than a reuse of the gutter's or the patch viewer's:
/// those say what happened to a line of a file, and this says what a number is
/// counting. They are drawn in the same two colours, which is a decision for
/// whoever draws them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CountKind {
    Added,
    Removed,
}

/// What stands in a count cell for a change whose lines were not counted: a
/// binary file, one too large to read, or a whole untracked directory.
///
/// Not a blank, because a blank cell and a cell holding zero look like the
/// same claim about a file, and they are not.
const UNCOUNTED: &str = "·";

/// Which side of the index a row stands on.
///
/// It is what a row means rather than where it was printed: staging a row that
/// is already staged should be understood as the no-op it is, and a file that
/// is both staged and edited again genuinely occupies a row on each side.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusSide {
    Staged,
    Unstaged,
}

impl StatusSide {
    /// The pair of trees whose difference a row on this side reports.
    const fn scope(self) -> DiffScope {
        match self {
            Self::Staged => DiffScope::Staged,
            Self::Unstaged => DiffScope::Unstaged,
        }
    }
}

/// The file a row acts on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusEntry {
    /// Repository-relative, exactly as Git spelled it.
    pub path: PathBuf,
    /// The source of a rename, when Git reported one. Actions that move the
    /// index must include both endpoints even though opening and diffing the
    /// row follow `path`, where the file now lives.
    pub original_path: Option<PathBuf>,
    pub side: StatusSide,
}

impl StatusEntry {
    /// Every repository-relative path whose index entry belongs to this row.
    pub fn mutation_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.original_path.iter().chain(std::iter::once(&self.path))
    }
}

/// Where a row's two counts sit inside its text, as character ranges.
///
/// Ranges rather than the numbers again: by the time a frontend has the row it
/// has one string, and what it needs to know is which part of it is the
/// addition and which the removal. Nothing here says what colour either one
/// is — that belongs to whoever is drawing, and to the theme it is drawing
/// with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CountColumns {
    pub added: Range<usize>,
    pub removed: Range<usize>,
}

impl CountColumns {
    /// Which of the two counts, if either, covers one character of the row.
    pub fn kind_at(&self, column: usize) -> Option<CountKind> {
        if self.added.contains(&column) {
            Some(CountKind::Added)
        } else if self.removed.contains(&column) {
            Some(CountKind::Removed)
        } else {
            None
        }
    }
}

/// One row of the list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusRow {
    pub text: String,
    /// `None` on headings and blank rows, which no key acts on.
    pub entry: Option<StatusEntry>,
    /// Where this row's counts are, absent on every row that has none: the
    /// headings, the blank rows, a file whose lines could not be counted, and
    /// every row of a list that has no counts at all.
    pub counts: Option<CountColumns>,
}

impl StatusRow {
    fn heading(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            entry: None,
            counts: None,
        }
    }

    fn blank() -> Self {
        Self::heading("")
    }
}

/// One file's row before it knows how wide the list's count column is.
///
/// The width is a property of the whole list rather than of any row in it, so
/// a row cannot be written out until every other row has been seen. Keeping
/// the parts apart until then is what lets the numbers line up without any of
/// this needing to know how wide a pane is.
struct FileRow {
    marker: char,
    name: String,
    entry: StatusEntry,
    /// What Git counted for this file on this side, absent where it could not.
    stats: Option<LineStats>,
}

impl FileRow {
    fn new(marker: char, file: &FileStatus, side: StatusSide, stats: &StatusStats) -> Self {
        let name = file.original_path.as_ref().map_or_else(
            || file.path.display().to_string(),
            |from| format!("{} → {}", from.display(), file.path.display()),
        );
        Self {
            marker,
            name,
            stats: stats.get(side.scope(), &file.path),
            entry: StatusEntry {
                path: file.path.clone(),
                // Porcelain type-2 records use the same source field for
                // renames and copies. Only a rename moves both endpoints;
                // mutating a copy's source would reach beyond this row.
                original_path: (file.index == FileState::Renamed
                    || file.worktree == FileState::Renamed)
                    .then(|| file.original_path.clone())
                    .flatten(),
                side,
            },
        }
    }

    /// Writes the row out against the column widths the whole list settled on.
    ///
    /// `None` widths mean nothing in the list was counted, and then there is no
    /// column at all: an empty one would be a stripe of dots down a list whose
    /// numbers were never available in the first place.
    fn render(self, widths: Option<(usize, usize)>) -> StatusRow {
        let Self {
            marker,
            name,
            entry,
            stats,
        } = self;
        let (text, counts) = match widths {
            Some((added_width, removed_width)) => {
                let (added, removed) = cells(stats);
                // The ranges are measured while the row is built rather than
                // searched for afterwards: the padding is what makes the
                // column align, and a reader of the finished string could not
                // tell a padded number from a name that happens to start with
                // a digit.
                let mut text = format!("  {marker}  ");
                let added_at = column(&text) + added_width - column(&added);
                text.push_str(&format!("{added:>added_width$}  "));
                let removed_at = column(&text) + removed_width - column(&removed);
                text.push_str(&format!("{removed:>removed_width$}  {name}"));
                let counts = stats.is_some().then(|| CountColumns {
                    added: added_at..added_at + column(&added),
                    removed: removed_at..removed_at + column(&removed),
                });
                (text, counts)
            }
            None => (format!("  {marker} {name}"), None),
        };
        StatusRow {
            text,
            entry: Some(entry),
            counts,
        }
    }
}

/// How many characters into a row something is, which is the unit every
/// offset in a buffer is counted in.
fn column(text: &str) -> usize {
    text.chars().count()
}

/// The two count cells of one row, as the text that stands in each.
fn cells(stats: Option<LineStats>) -> (String, String) {
    stats.map_or_else(
        || (UNCOUNTED.to_owned(), UNCOUNTED.to_owned()),
        |stats| (format!("+{}", stats.added), format!("-{}", stats.removed)),
    )
}

/// Projects a repository status into the rows of the changed-file list.
///
/// Conflicts come first because nothing else can be finished until they are
/// resolved, then what a commit would take, then what it would not, then what
/// Git is not yet tracking at all. A file that is both staged and edited again
/// appears twice, which is not a duplicate: those are two different changes,
/// and they are staged and unstaged separately.
pub fn status_rows(status: &RepositoryStatus, stats: &StatusStats) -> Vec<StatusRow> {
    let conflicted = status
        .files
        .iter()
        .filter(|file| file.is_conflicted())
        .map(|file| FileRow::new('U', file, StatusSide::Unstaged, stats));
    let staged = status
        .files
        .iter()
        .filter(|file| file.is_staged() && !file.is_conflicted())
        .map(|file| FileRow::new(file.index.marker(), file, StatusSide::Staged, stats));
    let unstaged = status
        .files
        .iter()
        .filter(|file| {
            !file.is_conflicted()
                && !file.is_untracked()
                && !matches!(file.worktree, FileState::Unmodified)
        })
        .map(|file| FileRow::new(file.worktree.marker(), file, StatusSide::Unstaged, stats));
    let untracked = status
        .files
        .iter()
        .filter(|file| file.is_untracked())
        .map(|file| FileRow::new('?', file, StatusSide::Unstaged, stats));

    let sections = [
        ("Conflicted", conflicted.collect::<Vec<_>>()),
        ("Staged", staged.collect()),
        ("Not staged", unstaged.collect()),
        ("Untracked", untracked.collect()),
    ];

    let widths = column_widths(&sections);
    // The total is summed over the rows rather than over what Git counted,
    // so the number in the heading is the sum of the numbers underneath it
    // even where one file is counted twice for having a row on each side.
    let total = widths.map(|_| {
        sections
            .iter()
            .flat_map(|(_, section)| section)
            .filter_map(|row| row.stats)
            .fold(LineStats::default(), LineStats::sum)
    });

    let mut rows = vec![StatusRow::heading(heading(status, &sections, total))];
    let mut empty = true;
    for (title, section) in sections {
        if section.is_empty() {
            continue;
        }
        empty = false;
        rows.push(StatusRow::blank());
        rows.push(StatusRow::heading(title));
        rows.extend(section.into_iter().map(|row| row.render(widths)));
    }
    if empty {
        rows.push(StatusRow::blank());
        rows.push(StatusRow::heading("working tree clean"));
    }
    rows
}

/// How wide the two count columns have to be for the whole list, or `None`
/// when there is nothing to count.
///
/// One width for the list rather than one per section: the sections are read
/// as one list, and numbers that stepped left and right between them would be
/// harder to compare than numbers with more space around them than they need.
fn column_widths(sections: &[(&str, Vec<FileRow>)]) -> Option<(usize, usize)> {
    let rows = || sections.iter().flat_map(|(_, section)| section);
    if !rows().any(|row| row.stats.is_some()) {
        return None;
    }
    Some(rows().fold((0, 0), |(added_width, removed_width), row| {
        let (added, removed) = cells(row.stats);
        (
            added_width.max(added.chars().count()),
            removed_width.max(removed.chars().count()),
        )
    }))
}

/// The first row: where `HEAD` is, and how much is in each section below.
///
/// Deliberately not the status line's `+2 ~3 -1`. That form is compact because
/// a status line has no room, and it counts what happened to each file rather
/// than which section it is in — so a staged modification reads as "modified"
/// there, directly above a heading that calls it staged. Here there is room to
/// say which, and the numbers agree with the sections they describe.
///
/// The line total closes it, on the same terms: it is the sum of the numbers
/// in the rows below, so a file that is staged and edited again contributes
/// both of its changes exactly as it occupies both of its rows.
fn heading(
    status: &RepositoryStatus,
    sections: &[(&str, Vec<FileRow>)],
    total: Option<LineStats>,
) -> String {
    let mut heading = format!("# {}", status.head.label());
    let Divergence { ahead, behind } = status.divergence;
    if ahead > 0 {
        heading.push_str(&format!(" ↑{ahead}"));
    }
    if behind > 0 {
        heading.push_str(&format!(" ↓{behind}"));
    }
    let mut counts = sections
        .iter()
        .filter(|(_, section)| !section.is_empty())
        .map(|(title, section)| format!("{} {}", section.len(), title.to_lowercase()))
        .collect::<Vec<_>>();
    if let Some(total) = total {
        counts.push(format!("+{} -{}", total.added, total.removed));
    }
    if !counts.is_empty() {
        heading.push_str(&format!(" · {}", counts.join(" · ")));
    }
    heading
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Head;
    use std::path::Path;

    fn status(files: Vec<FileStatus>) -> RepositoryStatus {
        RepositoryStatus {
            head: Head::Branch("main".to_owned()),
            upstream: None,
            divergence: Divergence::default(),
            files,
        }
    }

    fn file(path: &str, index: FileState, worktree: FileState) -> FileStatus {
        FileStatus {
            path: PathBuf::from(path),
            original_path: None,
            index,
            worktree,
        }
    }

    fn text(rows: &[StatusRow]) -> Vec<&str> {
        rows.iter().map(|row| row.text.as_str()).collect()
    }

    /// The list as it reads when nothing was counted, which is what every test
    /// about grouping and identity is about.
    fn status_rows(status: &RepositoryStatus) -> Vec<StatusRow> {
        super::status_rows(status, &StatusStats::default())
    }

    #[test]
    fn rows_are_grouped_by_what_a_commit_would_take() {
        let rows = status_rows(&status(vec![
            file("staged.rs", FileState::Modified, FileState::Unmodified),
            file("edited.rs", FileState::Unmodified, FileState::Modified),
            file("new.rs", FileState::Added, FileState::Unmodified),
            file("stray.rs", FileState::Untracked, FileState::Untracked),
        ]));

        assert_eq!(
            text(&rows),
            vec![
                "# main · 2 staged · 1 not staged · 1 untracked",
                "",
                "Staged",
                "  M staged.rs",
                "  A new.rs",
                "",
                "Not staged",
                "  M edited.rs",
                "",
                "Untracked",
                "  ? stray.rs",
            ]
        );
    }

    /// The numbers sit in one column for the whole list, wide enough for the
    /// largest of them, and the heading totals exactly what is below it.
    #[test]
    fn counts_are_one_aligned_column_summed_in_the_heading() {
        let mut stats = StatusStats::default();
        stats.insert(DiffScope::Staged, "staged.rs", LineStats::new(82, 12));
        stats.insert(DiffScope::Staged, "new.rs", LineStats::new(7, 0));
        stats.insert(DiffScope::Unstaged, "edited.rs", LineStats::new(3, 116));
        stats.insert(DiffScope::Unstaged, "stray.rs", LineStats::new(20, 0));

        let rows = super::status_rows(
            &status(vec![
                file("staged.rs", FileState::Modified, FileState::Unmodified),
                file("edited.rs", FileState::Unmodified, FileState::Modified),
                file("new.rs", FileState::Added, FileState::Unmodified),
                file("stray.rs", FileState::Untracked, FileState::Untracked),
            ]),
            &stats,
        );

        assert_eq!(
            text(&rows),
            vec![
                "# main · 2 staged · 1 not staged · 1 untracked · +112 -128",
                "",
                "Staged",
                "  M  +82   -12  staged.rs",
                "  A   +7    -0  new.rs",
                "",
                "Not staged",
                "  M   +3  -116  edited.rs",
                "",
                "Untracked",
                "  ?  +20    -0  stray.rs",
            ]
        );
    }

    /// The columns a frontend paints have to land on the numbers themselves,
    /// padding and all, or a colour would sit beside a count rather than on it.
    #[test]
    fn each_count_reports_where_it_sits_in_its_row() {
        let mut stats = StatusStats::default();
        stats.insert(DiffScope::Staged, "staged.rs", LineStats::new(82, 12));
        stats.insert(DiffScope::Unstaged, "edited.rs", LineStats::new(3, 7));

        let rows = super::status_rows(
            &status(vec![
                file("staged.rs", FileState::Modified, FileState::Unmodified),
                file("edited.rs", FileState::Unmodified, FileState::Modified),
                file("logo.png", FileState::Unmodified, FileState::Modified),
            ]),
            &stats,
        );

        let cell = |row: &StatusRow, range: Range<usize>| {
            row.text
                .chars()
                .skip(range.start)
                .take(range.len())
                .collect::<String>()
        };
        let counted = |row: &StatusRow| {
            let counts = row.counts.clone().expect("this row was counted");
            (
                cell(row, counts.added.clone()),
                cell(row, counts.removed.clone()),
            )
        };

        assert_eq!(counted(&rows[3]), ("+82".to_owned(), "-12".to_owned()));
        assert_eq!(counted(&rows[6]), ("+3".to_owned(), "-7".to_owned()));
        assert!(
            rows[7].counts.is_none(),
            "a row with no numbers has nothing to point at"
        );
        assert!(rows[0].counts.is_none(), "the heading is not a counted row");

        // A count knows which of the two it is at every character of it, and
        // nowhere else on the row.
        let counts = rows[3].counts.clone().unwrap();
        assert_eq!(counts.kind_at(counts.added.start), Some(CountKind::Added));
        assert_eq!(counts.kind_at(counts.added.end - 1), Some(CountKind::Added));
        assert_eq!(counts.kind_at(counts.added.end), None);
        assert_eq!(
            counts.kind_at(counts.removed.start),
            Some(CountKind::Removed)
        );
        assert_eq!(counts.kind_at(rows[3].text.chars().count() - 1), None);
    }

    /// Each side of a file that has a row on both is counted on its own, and a
    /// file whose lines could not be counted keeps its row without them.
    #[test]
    fn each_side_is_counted_separately_and_the_uncountable_shows_no_number() {
        let mut stats = StatusStats::default();
        stats.insert(DiffScope::Staged, "both.rs", LineStats::new(5, 1));
        stats.insert(DiffScope::Unstaged, "both.rs", LineStats::new(2, 0));

        let rows = super::status_rows(
            &status(vec![
                file("both.rs", FileState::Modified, FileState::Modified),
                file("logo.png", FileState::Unmodified, FileState::Modified),
            ]),
            &stats,
        );

        assert_eq!(
            text(&rows),
            vec![
                "# main · 1 staged · 2 not staged · +7 -1",
                "",
                "Staged",
                "  M  +5  -1  both.rs",
                "",
                "Not staged",
                "  M  +2  -0  both.rs",
                "  M   ·   ·  logo.png",
            ]
        );
    }

    /// Nothing counted is not a column of nothing: the list reads exactly as
    /// it did before there were counts to show.
    #[test]
    fn a_list_with_no_counts_keeps_its_plain_rows() {
        let rows = super::status_rows(
            &status(vec![file(
                "edited.rs",
                FileState::Unmodified,
                FileState::Modified,
            )]),
            &StatusStats::default(),
        );

        assert_eq!(
            text(&rows),
            vec!["# main · 1 not staged", "", "Not staged", "  M edited.rs"]
        );
    }

    /// Staged and then edited again is two changes, not one file listed twice
    /// by mistake: each row stages or unstages on its own.
    #[test]
    fn a_file_changed_on_both_sides_has_a_row_on_each() {
        let rows = status_rows(&status(vec![file(
            "both.rs",
            FileState::Modified,
            FileState::Modified,
        )]));

        let entries = rows
            .iter()
            .filter_map(|row| row.entry.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].side, StatusSide::Staged);
        assert_eq!(entries[1].side, StatusSide::Unstaged);
        assert!(
            entries
                .iter()
                .all(|entry| entry.path == Path::new("both.rs"))
        );
    }

    /// Nothing else can be finished until a conflict is, so it is read first.
    #[test]
    fn conflicts_come_before_everything_else() {
        let rows = status_rows(&status(vec![
            file("staged.rs", FileState::Modified, FileState::Unmodified),
            file("clash.rs", FileState::Conflicted, FileState::Conflicted),
        ]));

        assert_eq!(
            text(&rows),
            vec![
                "# main · 1 conflicted · 1 staged",
                "",
                "Conflicted",
                "  U clash.rs",
                "",
                "Staged",
                "  M staged.rs",
            ]
        );
        // Staging a conflicted path is how a resolution is recorded, so its
        // row acts as an unstaged one.
        assert_eq!(rows[3].entry.as_ref().unwrap().side, StatusSide::Unstaged);
    }

    #[test]
    fn a_rename_names_both_ends_and_acts_on_the_new_one() {
        let mut renamed = file("after.rs", FileState::Renamed, FileState::Unmodified);
        renamed.original_path = Some(PathBuf::from("before.rs"));
        let rows = status_rows(&status(vec![renamed]));

        assert_eq!(rows[3].text, "  R before.rs → after.rs");
        assert_eq!(
            rows[3].entry.as_ref().unwrap().path,
            PathBuf::from("after.rs")
        );
        assert_eq!(
            rows[3]
                .entry
                .as_ref()
                .unwrap()
                .mutation_paths()
                .cloned()
                .collect::<Vec<_>>(),
            vec![PathBuf::from("before.rs"), PathBuf::from("after.rs")]
        );
    }

    #[test]
    fn a_copy_displays_its_source_but_only_acts_on_the_copy() {
        let mut copied = file("copy.rs", FileState::Copied, FileState::Unmodified);
        copied.original_path = Some(PathBuf::from("source.rs"));
        let rows = status_rows(&status(vec![copied]));

        assert_eq!(rows[3].text, "  C source.rs → copy.rs");
        assert_eq!(
            rows[3]
                .entry
                .as_ref()
                .unwrap()
                .mutation_paths()
                .cloned()
                .collect::<Vec<_>>(),
            vec![PathBuf::from("copy.rs")]
        );
    }

    #[test]
    fn both_rows_of_a_renamed_then_modified_file_keep_both_endpoints() {
        let mut renamed = file("after.rs", FileState::Renamed, FileState::Modified);
        renamed.original_path = Some(PathBuf::from("before.rs"));
        let rows = status_rows(&status(vec![renamed]));

        let entries = rows
            .iter()
            .filter_map(|row| row.entry.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 2);
        for entry in entries {
            assert_eq!(
                entry.mutation_paths().cloned().collect::<Vec<_>>(),
                vec![PathBuf::from("before.rs"), PathBuf::from("after.rs")]
            );
        }
    }

    #[test]
    fn a_clean_tree_says_so_and_offers_nothing_to_act_on() {
        let rows = status_rows(&status(Vec::new()));

        assert_eq!(text(&rows), vec!["# main", "", "working tree clean"]);
        assert!(rows.iter().all(|row| row.entry.is_none()));
    }

    /// Headings and blank rows are not files, so a key pressed on one must
    /// find nothing rather than act on a neighbour.
    #[test]
    fn only_file_rows_carry_an_entry() {
        let rows = status_rows(&status(vec![file(
            "edited.rs",
            FileState::Unmodified,
            FileState::Modified,
        )]));

        let carried = rows
            .iter()
            .map(|row| row.entry.is_some())
            .collect::<Vec<_>>();
        assert_eq!(carried, vec![false, false, false, true]);
    }

    /// The heading counts sections, not per-file states, so it never says
    /// "modified" directly above a heading that says "staged".
    #[test]
    fn the_heading_agrees_with_the_sections_below_it() {
        let rows = status_rows(&status(vec![file(
            "lorem.md",
            FileState::Modified,
            FileState::Unmodified,
        )]));

        assert_eq!(rows[0].text, "# main · 1 staged");
        // The status line's compact form counts the same file differently, and
        // that is correct there: it is a glance at what changed, not a
        // breakdown of where it sits.
        assert_eq!(
            status(vec![file(
                "lorem.md",
                FileState::Modified,
                FileState::Unmodified
            )])
            .summary(),
            "main ~1"
        );
    }

    #[test]
    fn the_heading_carries_upstream_drift() {
        let mut drifted = status(vec![file(
            "edited.rs",
            FileState::Unmodified,
            FileState::Modified,
        )]);
        drifted.divergence = Divergence {
            ahead: 2,
            behind: 1,
        };

        assert_eq!(status_rows(&drifted)[0].text, "# main ↑2 ↓1 · 1 not staged");
    }
}
