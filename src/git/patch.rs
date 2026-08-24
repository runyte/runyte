// SPDX-License-Identifier: MPL-2.0

//! Exact-byte unified patches and revision-safe partial application.

use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use crate::{
    hash::sha256_hex,
    workspace::{BufferId, BufferRevision},
};

use super::{DiffScope, GitError, Result, history::valid_object_id};

pub const MAX_PATCH_BYTES: usize = 4 * 1024 * 1024;
static NEXT_GUARD: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct BufferRevisionGuard {
    id: u64,
    valid: Arc<AtomicBool>,
}

impl BufferRevisionGuard {
    pub fn new() -> Self {
        Self {
            id: NEXT_GUARD.fetch_add(1, Ordering::Relaxed),
            valid: Arc::new(AtomicBool::new(true)),
        }
    }
    pub fn invalidate(&self) {
        self.valid.store(false, Ordering::Release);
    }
    pub fn is_valid(&self) -> bool {
        self.valid.load(Ordering::Acquire)
    }
    pub const fn id(&self) -> u64 {
        self.id
    }
}

impl Default for BufferRevisionGuard {
    fn default() -> Self {
        Self::new()
    }
}
impl PartialEq for BufferRevisionGuard {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for BufferRevisionGuard {}
impl Hash for BufferRevisionGuard {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RepositoryFingerprint {
    pub head: Option<String>,
    pub index: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PartialStageRequest {
    pub repository: PathBuf,
    pub fingerprint: RepositoryFingerprint,
    pub path: PathBuf,
    pub disk_sha256: String,
    pub buffer: Option<(BufferId, BufferRevision)>,
    pub guard: Option<BufferRevisionGuard>,
    pub scope: DiffScope,
    pub hunk: String,
    pub patch: Vec<u8>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PartialStageSelection {
    pub path: PathBuf,
    pub scope: DiffScope,
    pub buffer: Option<(BufferId, BufferRevision)>,
    pub guard: Option<BufferRevisionGuard>,
    pub hunk: Option<String>,
    pub lines: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchHunk {
    pub identity: String,
    pub header: String,
    pub patch: Vec<u8>,
    pub new_lines: Vec<usize>,
    pub new_start: usize,
    pub new_count: usize,
    pub independent_deletion: bool,
}

impl PatchHunk {
    pub fn intersects_new_range(&self, first: usize, last: usize) -> bool {
        if self.new_count == 0 {
            let anchor = self.new_start.max(1);
            first <= anchor && anchor <= last
        } else {
            first <= self.new_start.saturating_add(self.new_count - 1) && self.new_start <= last
        }
    }
}

pub fn parse_hunks(patch: &[u8]) -> Result<Vec<PatchHunk>> {
    if patch.len() > MAX_PATCH_BYTES {
        return Err(GitError::TooLarge {
            command: "Git patch".to_owned(),
            limit: MAX_PATCH_BYTES,
        });
    }
    if patch.contains(&0)
        || patch
            .windows(b"GIT binary patch".len())
            .any(|value| value == b"GIT binary patch")
    {
        return refused("binary patches cannot be partially staged");
    }
    let starts = line_starts(patch);
    let hunks = starts
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, start)| patch[*start..].starts_with(b"@@ "))
        .collect::<Vec<_>>();
    if hunks.is_empty() {
        return refused("this diff has no text hunks to stage");
    }
    let header_end = hunks[0].1;
    let header = &patch[..header_end];
    if !header.windows(7).any(|value| value == b"diff --") {
        return malformed("patch has no file header");
    }
    // Every hunk is reassembled onto this one file header, so a second file
    // in the same patch would be staged against the first file's path. A
    // stray `diff --git` in a hunk body is also caught by the line-prefix
    // check below, but refusing it here states the single-file contract
    // instead of relying on that.
    if starts
        .iter()
        .any(|start| *start >= header_end && patch[*start..].starts_with(b"diff --"))
    {
        return refused("multi-file patches cannot be partially staged; stage one file at a time");
    }
    for unsupported in [
        b"old mode ".as_slice(),
        b"new mode ".as_slice(),
        b"new file mode ".as_slice(),
        b"deleted file mode ".as_slice(),
        b"similarity index ".as_slice(),
        b"rename from ".as_slice(),
        b"rename to ".as_slice(),
        b"copy from ".as_slice(),
        b"copy to ".as_slice(),
    ] {
        if header
            .split(|byte| *byte == b'\n')
            .any(|line| line.starts_with(unsupported))
        {
            return refused(
                "file mode, type, rename, and copy metadata cannot be partially staged; use a whole-file action or Lazygit",
            );
        }
    }
    let mut result = Vec::new();
    for (position, (line_index, start)) in hunks.iter().copied().enumerate() {
        let end = hunks
            .get(position + 1)
            .map_or(patch.len(), |(_, start)| *start);
        let header_line_end = patch[start..end]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(end, |offset| start + offset + 1);
        let header_line = String::from_utf8_lossy(&patch[start..header_line_end])
            .trim_end()
            .to_owned();
        let (_, (new_start, new_count)) = parse_hunk_header(&header_line)?;
        let mut new_line = new_start;
        let mut new_lines = Vec::new();
        let mut change_added = false;
        let mut change_removed = false;
        let mut independent_deletion = false;
        for &line_start in starts.iter().skip(line_index + 1) {
            if line_start >= end {
                break;
            }
            match patch[line_start] {
                b'+' => {
                    change_added = true;
                    new_lines.push(new_line);
                    new_line += 1;
                }
                b' ' => {
                    independent_deletion |= change_removed && !change_added;
                    change_added = false;
                    change_removed = false;
                    new_line += 1;
                }
                b'-' => change_removed = true,
                b'\\' => {}
                _ => return malformed("hunk contains an unknown line prefix"),
            }
        }
        independent_deletion |= change_removed && !change_added;
        let mut exact = header.to_vec();
        exact.extend_from_slice(&patch[start..end]);
        result.push(PatchHunk {
            identity: sha256_hex(&patch[start..end]),
            header: header_line,
            patch: exact,
            new_lines,
            new_start,
            new_count,
            independent_deletion,
        });
    }
    Ok(result)
}

pub fn select_lines(hunk: &PatchHunk, first: usize, last: usize) -> Result<Vec<u8>> {
    if first == 0 || last < first {
        return malformed("selected line range is invalid");
    }
    let selected = hunk
        .new_lines
        .iter()
        .filter(|line| **line >= first && **line <= last)
        .count();
    if selected == 0 {
        return refused("the selection contains no added or modified lines");
    }
    if hunk.independent_deletion {
        return refused(
            "the hunk also contains an independent deletion outside the selected new lines; use hunk staging or Lazygit",
        );
    }
    if selected != hunk.new_lines.len() {
        return refused(
            "selected-line staging currently requires every changed new line in one hunk; use hunk staging or Lazygit for patch surgery",
        );
    }
    Ok(hunk.patch.clone())
}

pub fn valid_fingerprint(value: &RepositoryFingerprint) -> bool {
    value.head.as_deref().is_none_or(valid_object_id)
        && value.index.len() == 64
        && value.index.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_hunk_header(header: &str) -> Result<((usize, usize), (usize, usize))> {
    let mut fields = header.split_whitespace();
    if fields.next() != Some("@@") {
        return malformed("invalid hunk header");
    }
    let old = fields
        .next()
        .and_then(|field| field.strip_prefix('-'))
        .ok_or_else(|| malformed_error("invalid old hunk range"))?;
    let new = fields
        .next()
        .and_then(|field| field.strip_prefix('+'))
        .ok_or_else(|| malformed_error("invalid new hunk range"))?;
    let parse = |range: &str| {
        let mut values = range.split(',');
        let start = values
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| malformed_error("invalid hunk line number"))?;
        let count = values
            .next()
            .map(|value| value.parse::<usize>())
            .transpose()
            .map_err(|_| malformed_error("invalid hunk line count"))?
            .unwrap_or(1);
        if values.next().is_some() || (start == 0 && count != 0) {
            return Err(malformed_error("invalid hunk range"));
        }
        Ok((start, count))
    };
    Ok((parse(old)?, parse(new)?))
}

fn line_starts(bytes: &[u8]) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(bytes.iter().enumerate().filter_map(|(index, byte)| {
        (*byte == b'\n' && index + 1 < bytes.len()).then_some(index + 1)
    }));
    starts
}

fn malformed<T>(detail: &str) -> Result<T> {
    Err(malformed_error(detail))
}
fn malformed_error(detail: &str) -> GitError {
    GitError::Malformed {
        command: "unified Git patch".to_owned(),
        detail: detail.to_owned(),
    }
}
fn refused<T>(detail: &str) -> Result<T> {
    Err(GitError::Failed {
        command: "partial staging".to_owned(),
        code: None,
        stderr: detail.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_hunks_have_stable_identities_and_changed_new_lines() {
        let patch = b"diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,2 @@\n-old\n+new\n same\n@@ -5 +5 @@\n-x\n+y\n";
        let hunks = parse_hunks(patch).unwrap();
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].new_lines, [1]);
        assert_eq!(parse_hunks(patch).unwrap()[0].identity, hunks[0].identity);
        assert_eq!(select_lines(&hunks[0], 1, 1).unwrap(), hunks[0].patch);
        assert!(select_lines(&hunks[0], 2, 2).is_err());
    }

    #[test]
    fn deletion_only_and_no_newline_hunks_remain_exact_but_are_not_line_stageable() {
        let deleted =
            b"diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +0,0 @@\n-old\n";
        let hunk = parse_hunks(deleted).unwrap().remove(0);
        assert!(hunk.new_lines.is_empty());
        assert_eq!(hunk.patch, deleted);
        assert!(select_lines(&hunk, 1, 1).is_err());

        let no_newline = b"diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n";
        assert_eq!(parse_hunks(no_newline).unwrap()[0].patch, no_newline);
        assert!(
            parse_hunks(b"diff --git a/a b/a\nGIT binary patch\n@@ -1 +1 @@\n-x\n+y\n").is_err()
        );
        assert!(parse_hunks(
            b"diff --git a/a b/a\nold mode 100644\nnew mode 100755\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\n"
        )
        .is_err());
    }

    #[test]
    fn a_second_file_is_refused_rather_than_staged_against_the_first() {
        let patch = b"diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/b.txt b/b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-x\n+y\n";
        let error = parse_hunks(patch).unwrap_err().to_string();
        assert!(error.contains("multi-file"), "{error}");
    }

    #[test]
    fn deletion_only_hunks_intersect_their_post_image_anchor() {
        let patch = b"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\n@@ -10 +10,0 @@\n-delete\n";
        let hunks = parse_hunks(patch).unwrap();
        assert!(hunks[0].intersects_new_range(1, 10));
        assert!(hunks[1].new_lines.is_empty());
        assert!(hunks[1].intersects_new_range(1, 10));
        assert!(!hunks[1].intersects_new_range(1, 9));
    }

    #[test]
    fn selected_replacement_refuses_an_independent_deletion_in_the_same_hunk() {
        let patch = b"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1,4 +1,3 @@\n-old-a\n+new-a\n keep\n-delete-me\n keep\n";
        let hunk = parse_hunks(patch).unwrap().remove(0);
        assert!(hunk.independent_deletion);
        assert!(select_lines(&hunk, 1, 1).is_err());
    }
}
