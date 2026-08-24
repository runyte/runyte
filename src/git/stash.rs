// SPDX-License-Identifier: MPL-2.0

//! Stable, bounded stash identities and explicit mutation requests.

use super::{GitError, Result, history::valid_object_id};

pub const MAX_STASH_ENTRIES: usize = 200;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StashEntry {
    pub oid: String,
    pub selector: String,
    pub subject: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StashScope {
    TrackedWorktree,
    TrackedWorktreeAndIndex,
    TrackedAndUntracked,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum StashMutation {
    Create { name: String, scope: StashScope },
    Apply { oid: String },
    Drop { oid: String },
}

pub fn parse_stashes(output: &[u8]) -> Result<Vec<StashEntry>> {
    let mut fields = output.split(|byte| *byte == 0).collect::<Vec<_>>();
    if fields.last().is_some_and(|field| field.is_empty()) {
        fields.pop();
    }
    if fields.len() % 3 != 0 {
        return malformed("stash record does not contain three fields");
    }
    if fields.len() / 3 > MAX_STASH_ENTRIES {
        return Err(GitError::TooLarge {
            command: "git stash list".to_owned(),
            limit: MAX_STASH_ENTRIES,
        });
    }
    fields
        .chunks_exact(3)
        .map(|fields| {
            let oid = std::str::from_utf8(fields[0])
                .map_err(|_| malformed_error("stash object ID is not UTF-8"))?;
            if !valid_object_id(oid) {
                return malformed("stash object ID is invalid");
            }
            let selector = std::str::from_utf8(fields[1])
                .map_err(|_| malformed_error("stash selector is not UTF-8"))?;
            if !selector.starts_with("stash@{") || !selector.ends_with('}') {
                return malformed("stash selector is invalid");
            }
            Ok(StashEntry {
                oid: oid.to_owned(),
                selector: selector.to_owned(),
                subject: safe_subject(fields[2]),
            })
        })
        .collect()
}

fn safe_subject(value: &[u8]) -> String {
    String::from_utf8_lossy(value)
        .chars()
        .map(|character| {
            if character.is_control() {
                if matches!(character, '\n' | '\r' | '\t') {
                    ' '
                } else {
                    '�'
                }
            } else {
                character
            }
        })
        .take(4096)
        .collect()
}

fn malformed<T>(detail: &str) -> Result<T> {
    Err(malformed_error(detail))
}
fn malformed_error(detail: &str) -> GitError {
    GitError::Malformed {
        command: "git stash list".to_owned(),
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delimiter_safe_stashes_keep_object_identity() {
        let oid = "a".repeat(40);
        let entries =
            parse_stashes(format!("{oid}\0stash@{{0}}\0name λ\tline\0").as_bytes()).unwrap();
        assert_eq!(entries[0].oid, oid);
        assert_eq!(entries[0].subject, "name λ line");
        let escaped =
            parse_stashes(format!("{oid}\0stash@{{0}}\0bad\u{1b}[2J\nname\0").as_bytes()).unwrap();
        assert_eq!(escaped[0].subject, "bad�[2J name");
        assert!(parse_stashes(b"bad\0stash@{0}\0x\0").is_err());
    }
}
