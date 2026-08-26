// SPDX-License-Identifier: MPL-2.0

//! Typed Git worktree discovery values and porcelain parser.

use std::{ffi::OsString, path::PathBuf};

use super::{GitError, Repository, Result};

/// One checkout registered with a common repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: Option<String>,
    pub prunable: Option<String>,
    pub missing: bool,
    pub common_dir: PathBuf,
}

/// An explicit, atomic `git worktree add` request.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WorktreeCreate {
    pub destination: PathBuf,
    pub start: String,
    pub new_branch: Option<String>,
}

/// Parses `git worktree list --porcelain -z` without interpreting a path as
/// UTF-8. Textual Git identities remain strict so malformed refs cannot be
/// confused with a valid selectable branch.
pub fn parse_worktree_porcelain(repository: &Repository, output: &[u8]) -> Result<Vec<Worktree>> {
    let mut result = Vec::new();
    let mut current: Option<Worktree> = None;
    for field in output.split(|byte| *byte == 0) {
        if field.is_empty() {
            if let Some(worktree) = current.take() {
                result.push(worktree);
            }
            continue;
        }
        if let Some(path) = field.strip_prefix(b"worktree ") {
            if let Some(worktree) = current.take() {
                result.push(worktree);
            }
            let path = decode_path(path);
            current = Some(Worktree {
                missing: !path.exists(),
                path,
                head: None,
                branch: None,
                detached: false,
                bare: false,
                locked: None,
                prunable: None,
                common_dir: repository.common_dir().to_path_buf(),
            });
            continue;
        }
        let worktree = current.as_mut().ok_or_else(|| GitError::Malformed {
            command: "git worktree list --porcelain -z".to_owned(),
            detail: "worktree record has fields before its path".to_owned(),
        })?;
        if let Some(value) = field.strip_prefix(b"HEAD ") {
            let oid = text(value, "object id")?;
            // Git represents an unborn worktree with an all-zero object ID.
            // That is absence, not a commit someone can navigate to.
            worktree.head =
                (!oid.is_empty() && !oid.bytes().all(|byte| byte == b'0')).then_some(oid);
        } else if let Some(value) = field.strip_prefix(b"branch ") {
            worktree.branch = Some(text(value, "branch ref")?);
        } else if field == b"detached" {
            worktree.detached = true;
        } else if field == b"bare" {
            worktree.bare = true;
        } else if field == b"locked" {
            worktree.locked = Some(String::new());
        } else if let Some(value) = field.strip_prefix(b"locked ") {
            worktree.locked = Some(text(value, "lock reason")?);
        } else if field == b"prunable" {
            worktree.prunable = Some(String::new());
        } else if let Some(value) = field.strip_prefix(b"prunable ") {
            worktree.prunable = Some(text(value, "prune reason")?);
        } else {
            return Err(GitError::Malformed {
                command: "git worktree list --porcelain -z".to_owned(),
                detail: format!(
                    "unknown porcelain field `{}`",
                    String::from_utf8_lossy(field)
                ),
            });
        }
    }
    if let Some(worktree) = current {
        result.push(worktree);
    }
    if result.is_empty() {
        return Err(GitError::Malformed {
            command: "git worktree list --porcelain -z".to_owned(),
            detail: "Git returned no worktree records".to_owned(),
        });
    }
    Ok(result)
}

fn text(value: &[u8], field: &str) -> Result<String> {
    String::from_utf8(value.to_vec()).map_err(|_| GitError::Malformed {
        command: "git worktree list --porcelain -z".to_owned(),
        detail: format!("{field} is not UTF-8"),
    })
}

#[cfg(unix)]
fn decode_path(value: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(value.to_vec()))
}

#[cfg(not(unix))]
fn decode_path(value: &[u8]) -> PathBuf {
    PathBuf::from(OsString::from(String::from_utf8_lossy(value).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parses_linked_detached_locked_prunable_bare_and_non_utf8_paths() {
        let repository = Repository::with_common_dir("/repo", "/common");
        let mut input =
            b"worktree /repo\0HEAD 0123\0branch refs/heads/main\0\0worktree /tmp/odd-".to_vec();
        input.extend_from_slice(&[0xff]);
        input.extend_from_slice(
            b"\0HEAD abcd\0detached\0locked maintenance\0prunable missing gitdir\0\0worktree /bare\0bare\0\0",
        );
        let worktrees = parse_worktree_porcelain(&repository, &input).unwrap();
        assert_eq!(worktrees.len(), 3);
        assert_eq!(worktrees[0].branch.as_deref(), Some("refs/heads/main"));
        assert!(worktrees[1].detached);
        assert!(worktrees[1].missing);
        assert_eq!(worktrees[1].locked.as_deref(), Some("maintenance"));
        assert_eq!(worktrees[1].prunable.as_deref(), Some("missing gitdir"));
        assert!(worktrees[2].bare);
        #[cfg(unix)]
        assert_eq!(
            worktrees[1].path.as_os_str().as_encoded_bytes(),
            b"/tmp/odd-\xff"
        );
        assert!(
            worktrees
                .iter()
                .all(|worktree| worktree.common_dir == Path::new("/common"))
        );
    }

    #[test]
    fn parses_an_unborn_head_as_no_object_identity() {
        let repository = Repository::new("/repo");
        let worktrees = parse_worktree_porcelain(
            &repository,
            b"worktree /definitely-missing-runyte-worktree\0HEAD 0000000000000000000000000000000000000000\0branch refs/heads/new\0\0",
        )
        .unwrap();
        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].head, None);
        assert_eq!(worktrees[0].branch.as_deref(), Some("refs/heads/new"));
        assert!(worktrees[0].missing);
    }

    #[test]
    fn refuses_malformed_text_and_unknown_fields() {
        let repository = Repository::new("/repo");
        assert!(parse_worktree_porcelain(&repository, b"worktree /repo\0branch \xff\0\0").is_err());
        assert!(parse_worktree_porcelain(&repository, b"worktree /repo\0mystery yes\0\0").is_err());
    }
}
