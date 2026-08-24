// SPDX-License-Identifier: MPL-2.0

//! Bounded, typed Git history values and machine-delimited parsers.

use super::{GitError, Result};

pub const DEFAULT_LOG_PAGE_SIZE: usize = 10_000;
pub const MAX_LOG_PAGE_SIZE: usize = 10_000;
pub const MAX_COMMIT_SEARCH_RESULTS: usize = 5_000;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LogCursor {
    pub boundary: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LogRequest {
    pub cursor: Option<LogCursor>,
    pub limit: usize,
}

impl Default for LogRequest {
    fn default() -> Self {
        Self {
            cursor: None,
            limit: DEFAULT_LOG_PAGE_SIZE,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitSummary {
    pub oid: String,
    pub abbreviated: String,
    pub parents: Vec<String>,
    pub author: String,
    pub author_time: i64,
    /// Author date as `YYYY-MM-DD` in the commit's own timezone, taken from
    /// Git's `%as` rather than derived from `author_time`, so it reads the
    /// same as `git log --date=short` instead of shifting by a day in UTC.
    pub author_date: String,
    pub subject: String,
    pub decorations: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogPage {
    pub commits: Vec<CommitSummary>,
    pub next: Option<LogCursor>,
    /// Total pages reachable from `HEAD` at the time this page was read.
    pub total_pages: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitDetail {
    pub summary: CommitSummary,
    pub body: String,
    pub patch: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitSearchEntry {
    pub summary: CommitSummary,
    /// Full commit message, retained for the fuzzy-search haystack.
    pub message: String,
}

impl CommitSearchEntry {
    /// The text the commit picker fuzzy-matches a query against.
    ///
    /// A commit is worth finding by anything the row shows, not only by what
    /// its author wrote: the object id, the author, and the author date join
    /// the message in one haystack. The abbreviated id is a prefix of the
    /// full one, so only the full id has to appear for either to match.
    /// Fields are separated by spaces because the matcher scores a character
    /// after a space as the start of a word.
    pub fn haystack(&self) -> String {
        format!(
            "{} {} {} {}",
            self.message.trim_end(),
            self.summary.oid,
            self.summary.author,
            self.summary.author_date
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitSearchResult {
    pub commits: Vec<CommitSearchEntry>,
    pub limited: bool,
}

/// Parses the eight fixed NUL-delimited fields emitted by `git log -z`.
pub fn parse_log(output: &[u8]) -> Result<Vec<CommitSummary>> {
    let mut fields = output.split(|byte| *byte == 0).collect::<Vec<_>>();
    // `-z` adds exactly one record terminator. An empty final decoration is
    // the empty eighth field immediately before it and must be retained.
    if fields.last().is_some_and(|field| field.is_empty()) {
        fields.pop();
    }
    if fields.len() % 8 != 0 {
        return malformed("history record does not contain eight fields");
    }
    fields
        .chunks_exact(8)
        .map(|fields| {
            let oid = object_id(fields[0], "commit object")?;
            let abbreviated = text(fields[1], 16);
            if abbreviated.is_empty()
                || abbreviated.len() > 16
                || !abbreviated.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return malformed("abbreviated object id is invalid");
            }
            let parents = fields[2]
                .split(|byte| *byte == b' ')
                .filter(|parent| !parent.is_empty())
                .map(|parent| object_id(parent, "parent object"))
                .collect::<Result<Vec<_>>>()?;
            let author_time = strict_text(fields[4], "author time")?
                .parse::<i64>()
                .map_err(|_| GitError::Malformed {
                    command: "git log".to_owned(),
                    detail: "author time is not an integer".to_owned(),
                })?;
            let author_date = text(fields[5], 10);
            if !valid_short_date(&author_date) {
                return malformed("author date is not a YYYY-MM-DD date");
            }
            let decorations = text(fields[7], 8_192)
                .split(", ")
                .filter(|value| !value.is_empty())
                .take(32)
                .map(|value| value.chars().take(256).collect())
                .collect();
            Ok(CommitSummary {
                oid,
                abbreviated,
                parents,
                author: text(fields[3], 256),
                author_time,
                author_date,
                subject: text(fields[6], 4_096),
                decorations,
            })
        })
        .collect()
}

/// Parses the eight summary fields used by [`parse_log`] plus a full message.
pub fn parse_commit_search(output: &[u8]) -> Result<Vec<CommitSearchEntry>> {
    let mut fields = output.split(|byte| *byte == 0).collect::<Vec<_>>();
    if fields.last().is_some_and(|field| field.is_empty()) {
        fields.pop();
    }
    if fields.len() % 9 != 0 {
        return malformed("commit-search record does not contain nine fields");
    }
    fields
        .chunks_exact(9)
        .map(|fields| {
            let mut summary_record = Vec::new();
            for field in &fields[..8] {
                summary_record.extend_from_slice(field);
                summary_record.push(0);
            }
            let mut summaries = parse_log(&summary_record)?;
            Ok(CommitSearchEntry {
                summary: summaries.remove(0),
                message: text(fields[8], 16_384),
            })
        })
        .collect()
}

/// Whether a value is the `YYYY-MM-DD` shape Git's `%as` emits.
fn valid_short_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && [0, 1, 2, 3, 5, 6, 8, 9]
            .iter()
            .all(|index| bytes[*index].is_ascii_digit())
}

pub fn valid_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn object_id(value: &[u8], field: &str) -> Result<String> {
    let value = strict_text(value, field)?;
    if !valid_object_id(&value) {
        return malformed(&format!("{field} is not a full object id"));
    }
    Ok(value)
}

fn strict_text(value: &[u8], field: &str) -> Result<String> {
    String::from_utf8(value.to_vec()).map_err(|_| GitError::Malformed {
        command: "git log".to_owned(),
        detail: format!("{field} is not UTF-8"),
    })
}

fn text(value: &[u8], limit: usize) -> String {
    String::from_utf8_lossy(value).chars().take(limit).collect()
}

fn malformed<T>(detail: &str) -> Result<T> {
    Err(GitError::Malformed {
        command: "git log".to_owned(),
        detail: detail.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_pages_default_to_the_full_editor_search_window() {
        let request = LogRequest::default();
        assert_eq!(request.limit, 10_000);
        assert_eq!(request.limit, MAX_LOG_PAGE_SIZE);
    }

    #[test]
    fn parses_merges_decorations_and_lossy_display_fields_without_losing_identity() {
        let first = "0123456789012345678901234567890123456789";
        let parent = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let merge = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let mut input = format!(
            "{first}\0{short}\0{parent} {merge}\0A",
            short = &first[..12]
        )
        .into_bytes();
        input.extend_from_slice(
            b"\xffuthor\0-12\x001969-12-31\0subject\xff\0HEAD -> main, tag: v1\0",
        );
        let commits = parse_log(&input).unwrap();
        assert_eq!(commits[0].oid, first);
        assert_eq!(commits[0].parents, [parent, merge]);
        assert_eq!(commits[0].author_time, -12);
        assert_eq!(commits[0].author_date, "1969-12-31");
        assert_eq!(commits[0].decorations, ["HEAD -> main", "tag: v1"]);
        assert!(commits[0].author.contains('\u{fffd}'));
        assert!(commits[0].subject.contains('\u{fffd}'));
    }

    #[test]
    fn malformed_field_counts_and_object_ids_are_refused() {
        assert!(parse_log(b"one\0two\0").is_err());
        assert!(parse_log(b"nope\0abc\0\0author\x001\x002026-08-12\0subject\0\0").is_err());
        // A well-formed record whose date is not YYYY-MM-DD is refused rather
        // than displayed as an unexplained column.
        assert!(
            parse_log(
                b"0123456789012345678901234567890123456789\x00012345678901\0\0author\x001\0nonsense\0subject\0\0"
            )
            .is_err()
        );
    }

    #[test]
    fn commit_search_keeps_the_full_message_beside_typed_summary_fields() {
        let oid = "0123456789012345678901234567890123456789";
        let input = format!(
            "{oid}\0{}\0\0Ada\01\02026-08-12\0Fix picker\0HEAD -> main\0Fix picker\n\nSearch the body too.\0",
            &oid[..12]
        );
        let commits = parse_commit_search(input.as_bytes()).unwrap();
        assert_eq!(commits[0].summary.subject, "Fix picker");
        assert!(commits[0].message.contains("Search the body too."));
    }

    #[test]
    fn the_search_haystack_carries_identity_beside_the_message() {
        let oid = "0123456789012345678901234567890123456789";
        let input = format!(
            "{oid}\0{}\0\0Ada\01\02026-08-12\0Fix picker\0HEAD -> main\0Fix picker\n\nSearch the body too.\n\0",
            &oid[..12]
        );
        let commits = parse_commit_search(input.as_bytes()).unwrap();
        assert_eq!(
            commits[0].haystack(),
            format!("Fix picker\n\nSearch the body too. {oid} Ada 2026-08-12")
        );
    }
}
