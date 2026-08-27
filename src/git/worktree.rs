// SPDX-License-Identifier: MPL-2.0

//! Typed Git worktree discovery values and porcelain parser.

use std::{
    ffi::OsString,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

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

/// What a workspace directory's Git metadata says about it, read from files
/// rather than from `git`.
///
/// The session manager needs these for every remembered workspace, including
/// the stopped ones no host can answer for, and a listing may hold hundreds of
/// rows. Spawning a process per row for three short facts would make opening
/// the manager wait on the process table, so this reads `HEAD`, the `.git`
/// link, and the repository config directly. Everything it cannot answer is
/// `None`; nothing here is a Git operation that could fail in a way worth
/// reporting.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceGitFacts {
    /// The checked-out branch, without its `refs/heads/` prefix. `None` when
    /// the checkout is detached or `HEAD` is unreadable.
    pub branch: Option<String>,
    /// The workspace directory itself, when it is a linked worktree rather
    /// than the repository's main checkout.
    pub worktree: Option<PathBuf>,
    /// The URL of `origin`, or of the first remote configured when there is no
    /// `origin`. `None` when the repository has no remote.
    pub remote: Option<String>,
}

/// The largest `HEAD` or `.git` link file this reads. Both hold one short
/// line; anything larger is not the file this is looking for.
const MAX_GIT_LINK_BYTES: u64 = 4 * 1024;
/// The largest repository config this parses. A config this size is already
/// far past anything Git writes, and the remote URL is not worth an unbounded
/// read.
const MAX_GIT_CONFIG_BYTES: u64 = 256 * 1024;

/// Reads the branch, worktree, and remote of one workspace directory.
///
/// Returns `None` when the directory is not the top level of a working tree.
/// A directory below one is deliberately not walked upwards: a workspace is a
/// project root, and answering for its parent repository would label the row
/// with a branch that is not this workspace's own.
pub fn read_workspace_git_facts(project_root: &Path) -> Option<WorkspaceGitFacts> {
    let link = project_root.join(".git");
    let metadata = fs::metadata(&link).ok()?;
    let git_dir = if metadata.is_dir() {
        link
    } else {
        let target = read_gitdir_link(&link)?;
        if target.is_absolute() {
            target
        } else {
            project_root.join(target)
        }
    };
    let common_dir = read_common_dir(&git_dir);
    // A `.git` file is not by itself a linked worktree. A submodule has one,
    // and so does a repository created with `--separate-git-dir`, and both of
    // those are main checkouts of their own repository. What distinguishes a
    // linked worktree is the topology the gitfile points into: a private
    // directory whose `commondir` names a repository shared with other
    // checkouts. A main checkout is its own common directory however it was
    // laid out.
    let worktree = (git_dir != common_dir).then(|| project_root.to_path_buf());
    Some(WorkspaceGitFacts {
        branch: read_head_branch(&git_dir),
        worktree,
        remote: read_remote_url(&common_dir),
    })
}

/// Resolves the `gitdir: <path>` line a linked worktree's `.git` file holds.
fn read_gitdir_link(link: &Path) -> Option<PathBuf> {
    let contents = read_bounded(link, MAX_GIT_LINK_BYTES)?;
    let value = contents.strip_prefix(b"gitdir:")?;
    let value = trim_ascii_bytes(value);
    (!value.is_empty()).then(|| decode_path(value))
}

/// The directory shared with every other worktree of the same repository.
///
/// A linked worktree's private Git directory names it in `commondir`, usually
/// relatively. The main checkout is its own common directory.
fn read_common_dir(git_dir: &Path) -> PathBuf {
    let Some(contents) = read_bounded(&git_dir.join("commondir"), MAX_GIT_LINK_BYTES) else {
        return git_dir.to_path_buf();
    };
    let value = trim_ascii_bytes(&contents);
    if value.is_empty() {
        return git_dir.to_path_buf();
    }
    let target = decode_path(value);
    let common = if target.is_absolute() {
        target
    } else {
        git_dir.join(target)
    };
    // `commondir` may spell the gitdir itself — `.` is the usual form — and
    // that is a main checkout saying so, not a link to somewhere else. The
    // comparison that decides is a path one, so both sides are resolved.
    let resolved = common.canonicalize().unwrap_or(common);
    if resolved
        == git_dir
            .canonicalize()
            .unwrap_or_else(|_| git_dir.to_path_buf())
    {
        git_dir.to_path_buf()
    } else {
        resolved
    }
}

/// The branch `HEAD` points at, or `None` for a detached or unborn checkout.
fn read_head_branch(git_dir: &Path) -> Option<String> {
    let contents = read_bounded(&git_dir.join("HEAD"), MAX_GIT_LINK_BYTES)?;
    let value = contents.strip_prefix(b"ref:")?;
    let value = trim_ascii_bytes(value);
    let name = String::from_utf8(value.to_vec()).ok()?;
    let name = name.strip_prefix("refs/heads/").unwrap_or(&name);
    (!name.is_empty()).then(|| name.to_owned())
}

/// The URL of `origin`, falling back to the first remote the config names.
///
/// This is a deliberately small reader for one key rather than a Git config
/// implementation: it understands section headers, subsection names, comments,
/// and `name = value`, which is everything Git itself writes. Includes are not
/// followed, so a URL that only exists in an included file reads as absent
/// rather than as a guess.
fn read_remote_url(common_dir: &Path) -> Option<String> {
    let contents = read_bounded(&common_dir.join("config"), MAX_GIT_CONFIG_BYTES)?;
    let contents = String::from_utf8(contents).ok()?;
    let mut remote: Option<String> = None;
    let mut first: Option<String> = None;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            remote = parse_remote_section(header);
            continue;
        }
        let Some(name) = remote.as_deref() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("url") {
            continue;
        }
        let value = value.trim().trim_matches('"');
        if value.is_empty() {
            continue;
        }
        if name == "origin" {
            return Some(value.to_owned());
        }
        first.get_or_insert_with(|| value.to_owned());
    }
    first
}

/// The remote a `[remote "name"]` header names, or `None` for any other
/// section.
fn parse_remote_section(header: &str) -> Option<String> {
    let rest = header.trim().strip_prefix("remote")?;
    let rest = rest.trim();
    // `[remote "origin"]` is what Git writes; `[remote.origin]` is the
    // equivalent one-token spelling a hand-edited config may use.
    if let Some(quoted) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        return Some(quoted.to_owned());
    }
    let named = rest.strip_prefix('.')?;
    (!named.is_empty()).then(|| named.to_owned())
}

fn read_bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > limit {
        return None;
    }
    let mut contents = Vec::new();
    file.take(limit).read_to_end(&mut contents).ok()?;
    Some(contents)
}

fn trim_ascii_bytes(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    &value[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A scratch directory that removes itself, so a Git-facts test never
    /// leaves a fake repository behind in the platform temporary directory.
    struct ScratchRoot(PathBuf);

    impl ScratchRoot {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};

            static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);
            let path = std::env::temp_dir().join(format!(
                "runyte-git-facts-{label}-{}-{}",
                std::process::id(),
                NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reads_branch_and_remote_from_a_main_checkout() {
        let root = ScratchRoot::new("main");
        let git = root.path().join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            git.join("config"),
            "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@example.com:me/project.git\n",
        )
        .unwrap();
        let facts = read_workspace_git_facts(root.path()).unwrap();
        assert_eq!(facts.branch.as_deref(), Some("main"));
        assert_eq!(facts.worktree, None);
        assert_eq!(
            facts.remote.as_deref(),
            Some("git@example.com:me/project.git")
        );
    }

    #[test]
    fn reads_a_linked_worktree_through_its_gitdir_link_and_commondir() {
        let root = ScratchRoot::new("linked");
        let common = root.path().join("main/.git");
        let private = common.join("worktrees/feature");
        std::fs::create_dir_all(&private).unwrap();
        std::fs::write(
            common.join("config"),
            "[remote \"upstream\"]\n\turl = https://example.com/other.git\n",
        )
        .unwrap();
        std::fs::write(private.join("HEAD"), "ref: refs/heads/enh/render-space\n").unwrap();
        // Git writes `commondir` relative to the worktree's private directory.
        std::fs::write(private.join("commondir"), "../..\n").unwrap();
        let checkout = root.path().join("feature");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", private.display()),
        )
        .unwrap();
        let facts = read_workspace_git_facts(&checkout).unwrap();
        assert_eq!(facts.branch.as_deref(), Some("enh/render-space"));
        assert_eq!(facts.worktree.as_deref(), Some(checkout.as_path()));
        // No `origin`, so the first configured remote answers instead.
        assert_eq!(
            facts.remote.as_deref(),
            Some("https://example.com/other.git")
        );
    }

    /// A gitfile is not a linked worktree by itself. `--separate-git-dir` and
    /// submodules both give a main checkout one, and neither is a worktree of
    /// something else.
    #[test]
    fn a_gitfile_main_checkout_is_not_reported_as_a_linked_worktree() {
        let root = ScratchRoot::new("separate-git-dir");
        let git = root.path().join("elsewhere/project.git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            git.join("config"),
            "[remote \"origin\"]\n\turl = https://example.com/project.git\n",
        )
        .unwrap();
        let checkout = root.path().join("project");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", git.display()),
        )
        .unwrap();

        let facts = read_workspace_git_facts(&checkout).unwrap();
        assert_eq!(facts.branch.as_deref(), Some("main"));
        assert_eq!(
            facts.worktree, None,
            "a separate git directory is still a main checkout"
        );
        assert_eq!(
            facts.remote.as_deref(),
            Some("https://example.com/project.git")
        );

        // A submodule is the same shape: its gitdir lives under the superproject
        // but is the submodule repository's own common directory.
        let module = root.path().join("super/.git/modules/lib");
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(module.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let nested = root.path().join("super/lib");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            nested.join(".git"),
            format!("gitdir: {}\n", module.display()),
        )
        .unwrap();
        assert_eq!(read_workspace_git_facts(&nested).unwrap().worktree, None);
    }

    /// Git writes `commondir` only for a linked worktree, but a hand-written
    /// one naming the gitdir itself still describes a main checkout.
    #[test]
    fn a_commondir_naming_its_own_gitdir_is_a_main_checkout() {
        let root = ScratchRoot::new("self-commondir");
        let git = root.path().join("project.git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("commondir"), ".\n").unwrap();
        let checkout = root.path().join("project");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", git.display()),
        )
        .unwrap();

        assert_eq!(read_workspace_git_facts(&checkout).unwrap().worktree, None);
    }

    #[test]
    fn a_detached_checkout_has_no_branch_and_a_plain_directory_has_no_facts() {
        let root = ScratchRoot::new("detached");
        let git = root.path().join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(
            git.join("HEAD"),
            "9f1c0a1c0a1c0a1c0a1c0a1c0a1c0a1c0a1c0a1c\n",
        )
        .unwrap();
        let facts = read_workspace_git_facts(root.path()).unwrap();
        assert_eq!(facts.branch, None);
        assert_eq!(facts.remote, None);

        let plain = ScratchRoot::new("plain");
        assert_eq!(read_workspace_git_facts(plain.path()), None);
    }

    #[test]
    fn origin_wins_over_an_earlier_remote_and_comments_are_ignored() {
        let root = ScratchRoot::new("origin");
        let git = root.path().join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            git.join("config"),
            "[remote \"fork\"]\n\turl = https://example.com/fork.git\n; [remote \"decoy\"]\n[remote \"origin\"]\n\turl = https://example.com/origin.git\n",
        )
        .unwrap();
        let facts = read_workspace_git_facts(root.path()).unwrap();
        assert_eq!(
            facts.remote.as_deref(),
            Some("https://example.com/origin.git")
        );
    }

    #[test]
    fn refuses_malformed_text_and_unknown_fields() {
        let repository = Repository::new("/repo");
        assert!(parse_worktree_porcelain(&repository, b"worktree /repo\0branch \xff\0\0").is_err());
        assert!(parse_worktree_porcelain(&repository, b"worktree /repo\0mystery yes\0\0").is_err());
    }
}
