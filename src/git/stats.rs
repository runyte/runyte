// SPDX-License-Identifier: MPL-2.0

//! How many lines each change adds and removes.
//!
//! The counts are Git's own, from `--numstat`, rather than anything derived
//! from a buffer: the changed-file list describes what is on disk and in the
//! index, so the numbers beside a file have to be counted from the same two
//! trees Git compared to decide the file belonged in the list at all.
//!
//! They are held apart from [`RepositoryStatus`](super::RepositoryStatus)
//! because they are read by a separate command and are worth reading only
//! while something is showing them. A status with no counts is not a status
//! that is missing something; it is the ordinary case.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use super::{DiffScope, status::path_from_bytes};

/// What one change does to a file, in lines.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LineStats {
    pub added: usize,
    pub removed: usize,
}

impl LineStats {
    pub const fn new(added: usize, removed: usize) -> Self {
        Self { added, removed }
    }

    /// Sums two counts, saturating rather than wrapping.
    ///
    /// A total that wrapped would be a smaller number than one of the files it
    /// covers, which is worse than a total that stops rising.
    pub(super) const fn sum(self, other: Self) -> Self {
        Self {
            added: self.added.saturating_add(other.added),
            removed: self.removed.saturating_add(other.removed),
        }
    }
}

/// The counts behind one changed-file list.
///
/// Keyed by side as well as by path, because a file that is staged and then
/// edited again has two different changes and a row for each: one number
/// belongs beside the staged row and the other beside the unstaged one.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StatusStats {
    staged: BTreeMap<PathBuf, LineStats>,
    unstaged: BTreeMap<PathBuf, LineStats>,
}

impl StatusStats {
    fn side(&self, scope: DiffScope) -> &BTreeMap<PathBuf, LineStats> {
        match scope {
            DiffScope::Staged => &self.staged,
            DiffScope::Unstaged => &self.unstaged,
        }
    }

    fn side_mut(&mut self, scope: DiffScope) -> &mut BTreeMap<PathBuf, LineStats> {
        match scope {
            DiffScope::Staged => &mut self.staged,
            DiffScope::Unstaged => &mut self.unstaged,
        }
    }

    pub fn insert(&mut self, scope: DiffScope, path: impl Into<PathBuf>, stats: LineStats) {
        self.side_mut(scope).insert(path.into(), stats);
    }

    pub fn extend(
        &mut self,
        scope: DiffScope,
        entries: impl IntoIterator<Item = (PathBuf, LineStats)>,
    ) {
        self.side_mut(scope).extend(entries);
    }

    /// What Git counted for one path on one side, or `None` when it counted
    /// nothing there: a binary file, a path it was never asked about, or a
    /// side that has no diff to measure.
    pub fn get(&self, scope: DiffScope, path: &Path) -> Option<LineStats> {
        self.side(scope).get(path).copied()
    }

    /// Whether nothing at all was counted, which is what a caller checks
    /// before deciding to show a column of numbers.
    pub fn is_empty(&self) -> bool {
        self.staged.is_empty() && self.unstaged.is_empty()
    }
}

/// Reads one `git diff --numstat -z` document.
///
/// Each record is `<added> TAB <removed> TAB <path>`. A rename leaves the path
/// field empty and follows with the source and destination as two further
/// records; the destination is what is kept, because that is the path the
/// changed-file list names. Binary files, which Git counts as `-`, are absent
/// from the result rather than present as zero: no lines changed is a
/// different fact from lines that cannot be counted.
pub fn parse_numstat(output: &[u8]) -> Result<Vec<(PathBuf, LineStats)>, String> {
    let mut entries = Vec::new();
    let mut records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        let mut parts = record.splitn(3, |byte| *byte == b'\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(format!(
                "numstat record `{}` is missing fields",
                String::from_utf8_lossy(record)
            ));
        };
        let path = if path.is_empty() {
            // A rename: the source, then the destination.
            records
                .next()
                .ok_or_else(|| "a numstat rename has no source path".to_owned())?;
            records
                .next()
                .ok_or_else(|| "a numstat rename has no destination path".to_owned())?
        } else {
            path
        };
        let (Some(added), Some(removed)) = (count(added)?, count(removed)?) else {
            continue;
        };
        entries.push((path_from_bytes(path)?, LineStats::new(added, removed)));
    }
    Ok(entries)
}

/// One side of a record: a number, or `-` where Git could not count.
fn count(field: &[u8]) -> Result<Option<usize>, String> {
    if field == b"-" {
        return Ok(None);
    }
    std::str::from_utf8(field)
        .ok()
        .and_then(|digits| digits.parse().ok())
        .map(Some)
        .ok_or_else(|| format!("`{}` is not a line count", String::from_utf8_lossy(field)))
}

/// Counts the lines of a file that has no diff to read.
///
/// An untracked file is entirely new, so every line in it is added, but Git
/// will not say so: nothing it is being compared against exists. Text that
/// holds a NUL byte is treated as binary and left uncounted, matching what
/// `--numstat` itself reports for one.
pub fn count_new_lines(content: &[u8]) -> Option<LineStats> {
    if content.contains(&0) {
        return None;
    }
    let newlines = content.iter().filter(|byte| **byte == b'\n').count();
    let unterminated = usize::from(!content.is_empty() && !content.ends_with(b"\n"));
    Some(LineStats::new(newlines + unterminated, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a NUL-terminated document the way Git writes one.
    fn document(records: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for record in records {
            bytes.extend_from_slice(record.as_bytes());
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn ordinary_records_carry_both_counts() {
        assert_eq!(
            parse_numstat(&document(&["12\t3\tsrc/app.rs", "0\t9\tREADME.md"])).unwrap(),
            vec![
                (PathBuf::from("src/app.rs"), LineStats::new(12, 3)),
                (PathBuf::from("README.md"), LineStats::new(0, 9)),
            ]
        );
    }

    /// The destination is the path the changed-file list names, so a rename is
    /// counted under it rather than under where it came from.
    #[test]
    fn a_rename_is_counted_under_its_destination() {
        assert_eq!(
            parse_numstat(&document(&[
                "1\t0\t",
                "old.txt",
                "new.txt",
                "4\t2\tother.rs"
            ]))
            .unwrap(),
            vec![
                (PathBuf::from("new.txt"), LineStats::new(1, 0)),
                (PathBuf::from("other.rs"), LineStats::new(4, 2)),
            ]
        );
    }

    /// Zero changed lines and lines that cannot be counted are different
    /// answers, and only the first is a number to show.
    #[test]
    fn binary_files_are_left_out_rather_than_counted_as_zero() {
        assert_eq!(
            parse_numstat(&document(&["-\t-\tlogo.png", "0\t0\tmode-only.rs"])).unwrap(),
            vec![(PathBuf::from("mode-only.rs"), LineStats::new(0, 0))]
        );
    }

    #[test]
    fn a_record_without_fields_is_rejected() {
        assert!(parse_numstat(&document(&["12 3 src/app.rs"])).is_err());
        assert!(parse_numstat(&document(&["x\t3\tsrc/app.rs"])).is_err());
    }

    #[test]
    fn a_new_file_counts_every_line_as_added() {
        assert_eq!(
            count_new_lines(b"one\ntwo\n"),
            Some(LineStats::new(2, 0)),
            "a terminated last line is not a third line"
        );
        assert_eq!(count_new_lines(b"one\ntwo"), Some(LineStats::new(2, 0)));
        assert_eq!(count_new_lines(b""), Some(LineStats::new(0, 0)));
        assert_eq!(count_new_lines(b"binary\0content"), None);
    }

    #[test]
    fn counts_are_kept_per_side() {
        let mut stats = StatusStats::default();
        stats.insert(DiffScope::Staged, "both.rs", LineStats::new(5, 1));
        stats.insert(DiffScope::Unstaged, "both.rs", LineStats::new(2, 0));

        assert_eq!(
            stats.get(DiffScope::Staged, Path::new("both.rs")),
            Some(LineStats::new(5, 1))
        );
        assert_eq!(
            stats.get(DiffScope::Unstaged, Path::new("both.rs")),
            Some(LineStats::new(2, 0))
        );
        assert_eq!(stats.get(DiffScope::Staged, Path::new("other.rs")), None);
        assert!(!stats.is_empty());
        assert!(StatusStats::default().is_empty());
    }
}
