// SPDX-License-Identifier: MPL-2.0

//! Parsing of `git status --porcelain=v2 -z`.
//!
//! Version 2 of the porcelain format is used because it states the index and
//! working-tree sides of a change separately and gives rename sources their
//! own field. `-z` is not an optimization: it is the only form in which Git
//! hands over a path it did not have to quote, so a file named with a space, a
//! quote, or a newline arrives as the bytes it really is.
//!
//! The parser is pure. It takes the bytes Git wrote and returns values, which
//! is what makes every shape below testable without a repository on disk.

use std::path::PathBuf;

use super::{Divergence, FileState, FileStatus, Head, RepositoryStatus};

/// Reads a complete `--porcelain=v2 --branch -z` document.
///
/// The error is a human-readable detail; the caller names the command that
/// produced it.
pub fn parse(output: &[u8]) -> Result<RepositoryStatus, String> {
    let mut head = None;
    let mut unborn = false;
    let mut upstream = None;
    let mut divergence = Divergence::default();
    let mut files = Vec::new();

    let mut records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        match record.first() {
            Some(b'#') => {
                let header = std::str::from_utf8(record)
                    .map_err(|_| "a status header is not UTF-8".to_owned())?;
                let Some((key, value)) = header
                    .strip_prefix("# ")
                    .and_then(|header| header.split_once(' '))
                else {
                    // Unknown headers are ignored rather than rejected: a
                    // newer Git may add one, and none of them can invalidate
                    // the entries below.
                    continue;
                };
                match key {
                    "branch.oid" => unborn = value == "(initial)",
                    "branch.head" => head = Some(value.to_owned()),
                    "branch.upstream" => upstream = Some(value.to_owned()),
                    "branch.ab" => divergence = parse_divergence(value)?,
                    _ => {}
                }
            }
            Some(b'1') => files.push(ordinary(record)?),
            Some(b'2') => {
                let original = records
                    .next()
                    .ok_or_else(|| "a rename entry has no source path".to_owned())?;
                let mut status = renamed(record)?;
                status.original_path = Some(path_from_bytes(original)?);
                files.push(status);
            }
            Some(b'u') => files.push(unmerged(record)?),
            Some(b'?') => files.push(single(record, FileState::Untracked)?),
            Some(b'!') => files.push(single(record, FileState::Ignored)?),
            _ => return Err(format!("unknown status record `{}`", lossy(record))),
        }
    }

    let head = match head {
        Some(name) if name == "(detached)" => Head::Detached(
            // A detached HEAD reports its commit through branch.oid, which is
            // the only place the id appears. Without it the label would be a
            // literal "(detached)", so an absent oid is a parse failure.
            detached_oid(output).ok_or_else(|| "a detached HEAD has no commit".to_owned())?,
        ),
        Some(name) if unborn => Head::Unborn(name),
        Some(name) => Head::Branch(name),
        None => return Err("status reported no branch header".to_owned()),
    };

    Ok(RepositoryStatus {
        head,
        upstream,
        divergence,
        files,
    })
}

fn detached_oid(output: &[u8]) -> Option<String> {
    output
        .split(|byte| *byte == 0)
        .filter_map(|record| std::str::from_utf8(record).ok())
        .find_map(|record| record.strip_prefix("# branch.oid "))
        .map(str::to_owned)
}

fn parse_divergence(value: &str) -> Result<Divergence, String> {
    let (ahead, behind) = value
        .split_once(' ')
        .ok_or_else(|| format!("cannot read ahead/behind counts from `{value}`"))?;
    let count = |field: &str, sign: char| {
        field
            .strip_prefix(sign)
            .and_then(|digits| digits.parse::<usize>().ok())
            .ok_or_else(|| format!("cannot read an ahead/behind count from `{field}`"))
    };
    Ok(Divergence {
        ahead: count(ahead, '+')?,
        behind: count(behind, '-')?,
    })
}

/// `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`
fn ordinary(record: &[u8]) -> Result<FileStatus, String> {
    let (fields, path) = fields(record, 8)?;
    let (index, worktree) = states(fields[1])?;
    Ok(FileStatus {
        path: path_from_bytes(path)?,
        original_path: None,
        index,
        worktree,
    })
}

/// `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>`
fn renamed(record: &[u8]) -> Result<FileStatus, String> {
    let (fields, path) = fields(record, 9)?;
    let (index, worktree) = states(fields[1])?;
    Ok(FileStatus {
        path: path_from_bytes(path)?,
        original_path: None,
        index,
        worktree,
    })
}

/// `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`
///
/// Both sides of an unmerged path are conflicted regardless of the exact
/// stage codes: nothing about it can be staged or reverted until the merge is
/// finished, so a finer reading would be a distinction without a difference.
fn unmerged(record: &[u8]) -> Result<FileStatus, String> {
    let (_, path) = fields(record, 10)?;
    Ok(FileStatus {
        path: path_from_bytes(path)?,
        original_path: None,
        index: FileState::Conflicted,
        worktree: FileState::Conflicted,
    })
}

/// `? <path>` and `! <path>`.
fn single(record: &[u8], state: FileState) -> Result<FileStatus, String> {
    let (_, path) = fields(record, 1)?;
    Ok(FileStatus {
        path: path_from_bytes(path)?,
        original_path: None,
        index: state,
        worktree: state,
    })
}

/// Splits `count` space-delimited fields off the front, returning them and the
/// unsplit remainder.
///
/// The remainder is always a path, and a path is never split on spaces: Git
/// puts it last for exactly this reason.
fn fields(record: &[u8], count: usize) -> Result<(Vec<&[u8]>, &[u8]), String> {
    let mut fields = Vec::with_capacity(count);
    let mut rest = record;
    for _ in 0..count {
        let position = rest
            .iter()
            .position(|byte| *byte == b' ')
            .ok_or_else(|| format!("status record `{}` is missing fields", lossy(record)))?;
        fields.push(&rest[..position]);
        rest = &rest[position + 1..];
    }
    if rest.is_empty() {
        return Err(format!("status record `{}` has no path", lossy(record)));
    }
    Ok((fields, rest))
}

fn states(code: &[u8]) -> Result<(FileState, FileState), String> {
    let [index, worktree] = code else {
        return Err(format!("`{}` is not a status code pair", lossy(code)));
    };
    let read = |byte: u8| {
        FileState::from_code(byte).ok_or_else(|| format!("`{}` is not a status code", byte as char))
    };
    Ok((read(*index)?, read(*worktree)?))
}

/// Git's bytes become a path without passing through `String`.
///
/// On Unix a filename is bytes, and one that is not UTF-8 is still a file
/// somebody is editing. Elsewhere paths are Unicode by construction, so bytes
/// that are not UTF-8 are a genuine parse failure.
pub(super) fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf, String> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        std::str::from_utf8(bytes).map(PathBuf::from).map_err(|_| {
            format!(
                "status reported a path that is not UTF-8: `{}`",
                lossy(bytes)
            )
        })
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
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
    fn a_clean_branch_parses_to_its_name_and_no_files() {
        let status = parse(&document(&[
            "# branch.oid 6a4f1c2d",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +0 -0",
        ]))
        .unwrap();

        assert_eq!(status.head, Head::Branch("main".to_owned()));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!(status.divergence, Divergence::default());
        assert!(status.files.is_empty());
    }

    #[test]
    fn both_sides_of_an_ordinary_change_are_kept_apart() {
        let status = parse(&document(&[
            "# branch.oid 6a4f1c2d",
            "# branch.head main",
            "1 M. N... 100644 100644 100644 aaaa bbbb staged.rs",
            "1 .M N... 100644 100644 100644 aaaa bbbb unstaged.rs",
            "1 MM N... 100644 100644 100644 aaaa bbbb both.rs",
            "1 D. N... 100644 000000 000000 aaaa bbbb removed.rs",
        ]))
        .unwrap();

        let states = status
            .files
            .iter()
            .map(|file| (file.path.to_str().unwrap(), file.index, file.worktree))
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            vec![
                ("staged.rs", FileState::Modified, FileState::Unmodified),
                ("unstaged.rs", FileState::Unmodified, FileState::Modified),
                ("both.rs", FileState::Modified, FileState::Modified),
                ("removed.rs", FileState::Deleted, FileState::Unmodified),
            ]
        );
        assert!(status.files[0].is_staged());
        assert!(!status.files[1].is_staged());
    }

    /// The source path of a rename is its own NUL-terminated record, and it
    /// must not be mistaken for the next entry.
    #[test]
    fn a_rename_consumes_the_record_holding_its_source() {
        let status = parse(&document(&[
            "# branch.oid 6a4f1c2d",
            "# branch.head main",
            "2 R. N... 100644 100644 100644 aaaa bbbb R100 after.rs",
            "before.rs",
            "? stray.rs",
        ]))
        .unwrap();

        assert_eq!(status.files.len(), 2);
        assert_eq!(status.files[0].path, PathBuf::from("after.rs"));
        assert_eq!(
            status.files[0].original_path,
            Some(PathBuf::from("before.rs"))
        );
        assert_eq!(status.files[0].index, FileState::Renamed);
        assert_eq!(status.files[1].path, PathBuf::from("stray.rs"));
        assert!(status.files[1].is_untracked());
    }

    #[test]
    fn paths_keep_the_spaces_and_quotes_git_did_not_quote() {
        let status = parse(&document(&[
            "# branch.oid 6a4f1c2d",
            "# branch.head main",
            "1 .M N... 100644 100644 100644 aaaa bbbb some dir/a \"file\".rs",
            "? another one.txt",
        ]))
        .unwrap();

        assert_eq!(
            status.files[0].path,
            PathBuf::from("some dir/a \"file\".rs")
        );
        assert_eq!(status.files[1].path, PathBuf::from("another one.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_utf8_is_still_a_file() {
        use std::os::unix::ffi::OsStrExt;

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"# branch.oid 6a4f1c2d\0# branch.head main\0");
        bytes.extend_from_slice(b"1 .M N... 100644 100644 100644 aaaa bbbb bad-");
        bytes.push(0xff);
        bytes.extend_from_slice(b".rs\0");

        let status = parse(&bytes).unwrap();
        assert_eq!(
            status.files[0].path.as_os_str().as_bytes(),
            b"bad-\xff.rs".as_slice()
        );
    }

    #[test]
    fn unmerged_paths_are_conflicted_on_both_sides() {
        let status = parse(&document(&[
            "# branch.oid 6a4f1c2d",
            "# branch.head main",
            "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc clash.rs",
        ]))
        .unwrap();

        assert_eq!(status.files[0].path, PathBuf::from("clash.rs"));
        assert!(status.files[0].is_conflicted());
        assert_eq!(status.counts().conflicted, 1);
    }

    #[test]
    fn an_unborn_branch_is_not_a_branch_with_commits() {
        let status = parse(&document(&[
            "# branch.oid (initial)",
            "# branch.head main",
            "? first.rs",
        ]))
        .unwrap();

        assert_eq!(status.head, Head::Unborn("main".to_owned()));
    }

    #[test]
    fn a_detached_head_reports_the_commit_it_is_on() {
        let status = parse(&document(&[
            "# branch.oid 1a2b3c4d5e6f7a8b",
            "# branch.head (detached)",
        ]))
        .unwrap();

        assert_eq!(status.head, Head::Detached("1a2b3c4d5e6f7a8b".to_owned()));
        assert_eq!(status.head.label(), "@1a2b3c4");
    }

    #[test]
    fn ahead_and_behind_counts_are_read_from_the_branch_header() {
        let status = parse(&document(&[
            "# branch.oid 6a4f1c2d",
            "# branch.head main",
            "# branch.ab +3 -12",
        ]))
        .unwrap();

        assert_eq!(
            status.divergence,
            Divergence {
                ahead: 3,
                behind: 12
            }
        );
    }

    #[test]
    fn a_truncated_or_unknown_record_is_a_parse_failure() {
        assert!(parse(&document(&["# branch.head main", "1 .M N... 100644"])).is_err());
        assert!(parse(&document(&["# branch.head main", "x something"])).is_err());
        assert!(parse(&document(&["1 .M N... 100644 100644 100644 a b c.rs"])).is_err());
        // A rename whose source record never arrived.
        assert!(
            parse(&document(&[
                "# branch.head main",
                "2 R. N... 100644 100644 100644 aaaa bbbb R100 after.rs",
            ]))
            .is_err()
        );
    }

    /// Headers a future Git might add must not stop the entries being read.
    #[test]
    fn unknown_headers_are_ignored() {
        let status = parse(&document(&[
            "# branch.oid 6a4f1c2d",
            "# branch.head main",
            "# stash 3",
            "? stray.rs",
        ]))
        .unwrap();

        assert_eq!(status.files.len(), 1);
    }
}
