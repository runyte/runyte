// SPDX-License-Identifier: MPL-2.0

//! Typed parsing for `git blame --line-porcelain`.

use std::path::PathBuf;

use chrono::{DateTime, FixedOffset, Utc};

use super::{GitError, Result, history::valid_object_id};

pub const MAX_BLAME_INPUT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_BLAME_LINES: usize = 20_000;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BlameRequest {
    pub path: PathBuf,
    pub content: String,
    /// Inclusive, one-based source line window. `None` means the whole file.
    pub lines: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlameLine {
    pub oid: Option<String>,
    pub author: String,
    pub author_time: Option<i64>,
    /// Author date as `YYYY-MM-DD` in the commit's own timezone, computed
    /// from `author-time` and `author-tz` since `--line-porcelain` has no
    /// preformatted date field. Matches how [`super::history::CommitSummary::author_date`]
    /// reads, rather than shifting by a day under UTC or the local machine's
    /// timezone.
    pub author_date: Option<String>,
    pub summary: String,
    pub source_line: usize,
    pub text: String,
}

pub fn parse_blame(output: &[u8]) -> Result<Vec<BlameLine>> {
    let mut lines = output.split(|byte| *byte == b'\n').peekable();
    let mut result = Vec::new();
    while let Some(header) = lines.next() {
        if header.is_empty() {
            continue;
        }
        let mut fields = header.split(|byte| *byte == b' ');
        let oid = fields.next().unwrap_or_default();
        let _original = positive(fields.next(), "original line")?;
        let source_line = positive(fields.next(), "final line")?;
        // Porcelain emits the group length only on the first row in a run;
        // continuation rows contain exactly the first three fields.
        if let Some(group) = fields.next() {
            let _ = positive(Some(group), "group length")?;
        }
        if fields.next().is_some() || (!is_zero_oid(oid) && !valid_oid_bytes(oid)) {
            return malformed("line header has an invalid object identity");
        }
        let oid = (!is_zero_oid(oid)).then(|| String::from_utf8_lossy(oid).into_owned());
        let mut author = String::new();
        let mut author_time = None;
        let mut author_tz = None;
        let mut summary = String::new();
        let text = loop {
            let line = lines
                .next()
                .ok_or_else(|| malformed_error("line record ended before its content"))?;
            if let Some(content) = line.strip_prefix(b"\t") {
                break String::from_utf8_lossy(content).into_owned();
            }
            if let Some(value) = line.strip_prefix(b"author ") {
                author = bounded(value, 256);
            } else if let Some(value) = line.strip_prefix(b"author-time ") {
                author_time = Some(
                    String::from_utf8_lossy(value)
                        .parse()
                        .map_err(|_| malformed_error("author time is not an integer"))?,
                );
            } else if let Some(value) = line.strip_prefix(b"author-tz ") {
                author_tz = Some(bounded(value, 8));
            } else if let Some(value) = line.strip_prefix(b"summary ") {
                summary = bounded(value, 4_096);
            }
        };
        let author_date = author_time
            .zip(author_tz.as_deref())
            .and_then(|(time, tz)| author_date(time, tz));
        result.push(BlameLine {
            oid,
            author,
            author_time,
            author_date,
            summary,
            source_line,
            text,
        });
        if result.len() > MAX_BLAME_LINES {
            return Err(GitError::Failed {
                command: "git blame".to_owned(),
                code: None,
                stderr: format!("blame output is limited to {MAX_BLAME_LINES} lines"),
            });
        }
    }
    Ok(result)
}

fn positive(value: Option<&[u8]>, field: &str) -> Result<usize> {
    let value = value.ok_or_else(|| malformed_error(&format!("missing {field}")))?;
    let value = String::from_utf8_lossy(value)
        .parse::<usize>()
        .map_err(|_| malformed_error(&format!("{field} is not an integer")))?;
    if value == 0 {
        return malformed(&format!("{field} is zero"));
    }
    Ok(value)
}

fn valid_oid_bytes(value: &[u8]) -> bool {
    std::str::from_utf8(value).is_ok_and(valid_object_id)
}

fn is_zero_oid(value: &[u8]) -> bool {
    matches!(value.len(), 40 | 64) && value.iter().all(|byte| *byte == b'0')
}

/// Formats a `YYYY-MM-DD` date from `author-time` (Unix seconds) applied
/// through `author-tz` (e.g. `+0200`), so the date reads the same as
/// `git log --date=short` instead of shifting by a day in another timezone.
fn author_date(time: i64, tz: &str) -> Option<String> {
    let offset = parse_tz_offset(tz)?;
    let instant = DateTime::<Utc>::from_timestamp(time, 0)?;
    Some(
        instant
            .with_timezone(&offset)
            .format("%Y-%m-%d")
            .to_string(),
    )
}

fn parse_tz_offset(tz: &str) -> Option<FixedOffset> {
    let bytes = tz.as_bytes();
    if bytes.len() != 5 {
        return None;
    }
    let sign = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let hours: i32 = std::str::from_utf8(&bytes[1..3]).ok()?.parse().ok()?;
    let minutes: i32 = std::str::from_utf8(&bytes[3..5]).ok()?.parse().ok()?;
    FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))
}

fn bounded(value: &[u8], limit: usize) -> String {
    String::from_utf8_lossy(value).chars().take(limit).collect()
}

fn malformed<T>(detail: &str) -> Result<T> {
    Err(malformed_error(detail))
}

fn malformed_error(detail: &str) -> GitError {
    GitError::Malformed {
        command: "git blame --line-porcelain".to_owned(),
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_committed_and_live_uncommitted_lines() {
        let oid = "0123456789012345678901234567890123456789";
        let zero = "0000000000000000000000000000000000000000";
        let input = format!(
            "{oid} 1 1 1\nauthor Alice\nauthor-time 12\nsummary base\nfilename note.txt\n\tcommitted\n{zero} 2 2 1\nauthor Not Committed Yet\nsummary Version of note.txt from note.txt\nfilename note.txt\n\tlive λ\n"
        );
        let lines = parse_blame(input.as_bytes()).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].oid.as_deref(), Some(oid));
        assert_eq!(lines[0].author_time, Some(12));
        // No `author-tz` header in this fixture, so no date can be computed.
        assert_eq!(lines[0].author_date, None);
        assert_eq!(lines[1].oid, None);
        assert_eq!(lines[1].text, "live λ");
    }

    #[test]
    fn author_date_uses_the_commits_own_timezone() {
        let oid = "0123456789012345678901234567890123456789";
        // 2024-01-01T23:30:00Z, which is 2024-01-02 local under `+0200`:
        // the UTC calendar date alone would report the wrong day.
        let input = format!(
            "{oid} 1 1 1\nauthor Alice\nauthor-time 1704151800\nauthor-tz +0200\nsummary base\n\tcommitted\n"
        );
        let lines = parse_blame(input.as_bytes()).unwrap();
        assert_eq!(lines[0].author_date.as_deref(), Some("2024-01-02"));
    }

    #[test]
    fn malformed_headers_are_refused() {
        assert!(parse_blame(b"bad 1 1 1\n\ttext\n").is_err());
        assert!(parse_blame(b"0000000000000000000000000000000000000000 0 1 1\n\ttext\n").is_err());
    }

    #[test]
    fn accepts_group_continuation_headers_without_a_length() {
        let oid = "0123456789012345678901234567890123456789";
        let input = format!(
            "{oid} 1 1 3\nauthor Alice\nsummary three\n\tone\n{oid} 2 2\nauthor Alice\nsummary three\n\ttwo\n{oid} 3 3\nauthor Alice\nsummary three\n\tthree\n"
        );
        let lines = parse_blame(input.as_bytes()).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[2].text, "three");
    }
}
