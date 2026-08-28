// SPDX-License-Identifier: MPL-2.0

//! The one boundary between Runyte and Git.
//!
//! Git is an external executable here, never a library and never a shell
//! command line. Every call is an explicit argument vector run in a known
//! directory, its output is bounded before it is read into memory, and its
//! failures are values rather than prose the caller has to parse back out of a
//! message.
//!
//! Nothing in this module knows about buffers, panes, or drawing. The editor
//! asks which repository owns a directory, which files it reports changed, and
//! what a file looked like when it was last staged. Turning the last of those
//! into per-row marks is [`tracker`]'s job, and painting them belongs to a
//! frontend.

use std::{
    fmt,
    path::{Path, PathBuf},
};

pub mod blame;
pub mod branch_view;
pub mod cli;
pub mod diff;
pub mod history;
pub mod patch;
pub(crate) mod repository_lock;
pub mod service;
pub mod stash;
pub mod stats;
pub mod status;
pub mod tracker;
pub mod view;
pub mod worktree;

pub use blame::{BlameLine, BlameRequest, MAX_BLAME_INPUT_BYTES, MAX_BLAME_LINES, parse_blame};
pub(crate) use branch_view::display_path;
pub use branch_view::{BranchRow, branch_rows};
pub use cli::GitCliProvider;
pub use diff::{DiffLine, LineChange, RowChange, changed_rows, classify_line};
pub use history::{
    CommitDetail, CommitSearchEntry, CommitSearchResult, CommitSummary, DEFAULT_LOG_PAGE_SIZE,
    LogCursor, LogPage, LogRequest, MAX_COMMIT_SEARCH_RESULTS, MAX_LOG_PAGE_SIZE,
    parse_commit_search, parse_log,
};
pub use patch::{
    BufferRevisionGuard, MAX_PATCH_BYTES, PartialStageRequest, PartialStageSelection, PatchHunk,
    RepositoryFingerprint, parse_hunks, select_lines,
};
pub use service::{
    BlameSource, GitMutation, GitOperation, GitRequestId, GitResponse, GitService, GitServiceEvent,
    GitServiceHandle, GitServiceProgress, GitServiceState, RefreshSpec, RepositoryGeneration,
    RepositorySnapshot,
};
pub use stash::{MAX_STASH_ENTRIES, StashEntry, StashMutation, StashScope, parse_stashes};
pub use stats::{LineStats, StatusStats, count_new_lines, parse_numstat};
pub use tracker::GitTracker;
pub use view::{CountColumns, CountKind, StatusEntry, StatusRow, StatusSide, status_rows};
pub use worktree::{
    WorkspaceGitFacts, Worktree, WorktreeCreate, parse_worktree_porcelain, read_workspace_git_facts,
};

pub type Result<T> = std::result::Result<T, GitError>;

/// Why a Git call could not answer.
///
/// The variants exist so callers can decide without reading text.
/// [`GitError::Unavailable`] means every Git surface should degrade to absent
/// rather than report a failure; the rest are real errors worth showing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitError {
    /// No usable `git` executable, or one that could not be started.
    Unavailable { detail: String },
    /// The path exists but no repository owns it.
    NotARepository { path: PathBuf },
    /// Git ran and refused.
    Failed {
        command: String,
        code: Option<i32>,
        stderr: String,
    },
    /// A branch switch was refused because the index or working tree differs
    /// from `HEAD`.
    DirtyWorktree { files: usize },
    /// A branch and its upstream have both moved on, so neither one can reach
    /// the other by fast-forward.
    ///
    /// This is its own variant rather than a `Failed` carrying whatever Git
    /// printed because it is the one network refusal with a next step: the
    /// counts here are what a reconcile would have to replay, and the caller
    /// offers that rather than reporting a dead end.
    Diverged {
        branch: String,
        upstream: String,
        ahead: usize,
        behind: usize,
    },
    /// Git produced more output than the call was willing to hold.
    TooLarge { command: String, limit: usize },
    /// Git was still running when the call's deadline passed, and was stopped.
    ///
    /// Network operations and potentially large history/attribution reads
    /// carry deadlines. Ordinary local mutations finish or are cancelled.
    TimedOut { command: String, seconds: u64 },
    /// The owned command tree was stopped after the caller cancelled it.
    /// Mutating callers must treat the repository state as uncertain.
    Cancelled { command: String },
    /// Git succeeded but said something this parser does not understand.
    Malformed { command: String, detail: String },
    Io {
        action: &'static str,
        path: PathBuf,
        detail: String,
    },
}

impl GitError {
    /// Whether this means "there is no Git here" rather than "Git failed".
    pub const fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }

    /// A summary safe to put in a durable file.
    ///
    /// `Display` is written for the person reading the interaction line, so it
    /// quotes the argument vector and Git's stderr. Both are unbounded local
    /// content — a failing commit's argument vector contains the message that
    /// was just typed — and neither belongs in a diagnostic log. This names the
    /// refusal and keeps only fields that cannot carry document, typed, or
    /// subprocess text: exit status, byte limit, deadline, and counts.
    pub fn redacted(&self) -> String {
        match self {
            Self::Unavailable { .. } => "Git is unavailable".to_owned(),
            Self::NotARepository { .. } => "path is not in a Git repository".to_owned(),
            Self::Failed { code, .. } => match code {
                Some(code) => format!("Git refused with status {code}"),
                None => "Git refused".to_owned(),
            },
            Self::DirtyWorktree { files } => {
                format!("worktree differs from HEAD in {files} file(s)")
            }
            Self::Diverged { ahead, behind, .. } => {
                format!("branch and upstream diverged by {ahead} ahead and {behind} behind")
            }
            Self::TooLarge { limit, .. } => format!("Git output exceeded {limit} bytes"),
            Self::TimedOut { seconds, .. } => format!("Git timed out after {seconds}s"),
            Self::Cancelled { .. } => "Git was cancelled".to_owned(),
            Self::Malformed { .. } => "Git output could not be parsed".to_owned(),
            Self::Io { action, .. } => format!("Git {action} failed"),
        }
    }
}

impl fmt::Display for GitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { detail } => write!(formatter, "Git is unavailable: {detail}"),
            Self::NotARepository { path } => {
                write!(formatter, "{} is not in a Git repository", path.display())
            }
            Self::Failed {
                command,
                code,
                stderr,
            } => {
                write!(formatter, "`{command}` failed")?;
                if let Some(code) = code {
                    write!(formatter, " with status {code}")?;
                }
                if !stderr.is_empty() {
                    write!(formatter, ": {stderr}")?;
                }
                Ok(())
            }
            Self::DirtyWorktree { files } => write!(
                formatter,
                "cannot switch branches with uncommitted changes ({files} file{})",
                if *files == 1 { "" } else { "s" }
            ),
            Self::Diverged {
                branch,
                upstream,
                ahead,
                behind,
            } => write!(
                formatter,
                "{branch} and {upstream} have both moved on: {ahead} commit{} here, \
                 {behind} commit{} there",
                if *ahead == 1 { "" } else { "s" },
                if *behind == 1 { "" } else { "s" }
            ),
            Self::TooLarge { command, limit } => write!(
                formatter,
                "`{command}` produced more than {limit} bytes of output"
            ),
            Self::TimedOut { command, seconds } => write!(
                formatter,
                "`{command}` was still running after {seconds}s and was stopped"
            ),
            Self::Cancelled { command } => write!(formatter, "`{command}` was cancelled"),
            Self::Malformed { command, detail } => {
                write!(formatter, "cannot read the output of `{command}`: {detail}")
            }
            Self::Io {
                action,
                path,
                detail,
            } => write!(formatter, "cannot {action} {}: {detail}", path.display()),
        }
    }
}

impl std::error::Error for GitError {}

fn stale_deletion(target: &str) -> GitError {
    GitError::Failed {
        command: format!("delete {target}"),
        code: None,
        stderr: format!("the {target} changed after it was reviewed; review the deletion again"),
    }
}

fn typed_deletion_required(target: &str) -> GitError {
    GitError::Failed {
        command: format!("delete {target}"),
        code: None,
        stderr: format!("the {target} has unpublished history and needs typed confirmation"),
    }
}

/// A working tree Runyte can ask questions about.
///
/// Only the top level is kept: it is what every other call is resolved
/// against, and it is the one fact the editor needs in order to decide whether
/// a buffer belongs to this repository at all.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Repository {
    workdir: PathBuf,
    common_dir: PathBuf,
}

impl Repository {
    pub fn new(workdir: impl Into<PathBuf>) -> Self {
        let workdir = workdir.into();
        Self {
            common_dir: workdir.join(".git"),
            workdir,
        }
    }

    pub fn with_common_dir(workdir: impl Into<PathBuf>, common_dir: impl Into<PathBuf>) -> Self {
        Self {
            workdir: workdir.into(),
            common_dir: common_dir.into(),
        }
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// Directory shared by every linked worktree of this repository.
    pub fn common_dir(&self) -> &Path {
        &self.common_dir
    }

    /// The repository-relative form of `path`, or `None` when the path lies
    /// outside this working tree.
    ///
    /// Both sides are compared as given. Callers hand in paths that were
    /// already resolved the same way the working tree was.
    pub fn relative<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        path.strip_prefix(&self.workdir).ok()
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.relative(path).is_some()
    }
}

/// What `HEAD` currently names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Head {
    Branch(String),
    /// A branch that exists in name only, before the first commit.
    Unborn(String),
    /// Detached, holding the full object id.
    Detached(String),
}

/// The remote-tracking branch a local branch is configured to follow.
///
/// The drift is optional rather than zeroed because "in step with its upstream"
/// and "the upstream it names is gone" are different answers, and a reader
/// deciding whether to push needs to tell them apart.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Upstream {
    /// The short ref name, as `origin/main`.
    pub name: String,
    /// The remote the upstream lives on, as `origin`.
    ///
    /// Taken from Git rather than split off the front of `name`: a remote is
    /// what Git says it is, and reconstructing one by cutting at the first
    /// slash guesses at a boundary Git already knows.
    pub remote: String,
    /// The full ref on that remote, as `refs/heads/main`.
    pub reference: String,
    /// How far the local branch has drifted, or `None` when the upstream ref
    /// no longer exists.
    pub divergence: Option<Divergence>,
}

impl Upstream {
    /// The upstream a plain clone gives a branch: `origin/<branch>`.
    pub fn origin(branch: &str, divergence: Option<Divergence>) -> Self {
        Self {
            name: format!("origin/{branch}"),
            remote: "origin".to_owned(),
            reference: format!("refs/heads/{branch}"),
            divergence,
        }
    }
}

/// One local branch that can be checked out directly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    /// Registered worktree paths whose `HEAD` names this branch.
    ///
    /// This is a vector because Git can be explicitly told to check out one
    /// branch more than once. Paths remain operating-system values here; only
    /// the branch-list projection makes a lossy, escaped display string.
    pub checkouts: Vec<PathBuf>,
    /// What this branch tracks, absent when nothing is configured.
    pub upstream: Option<Upstream>,
    /// Whether every commit on this branch is already reachable from `HEAD`.
    ///
    /// Deleting an unmerged branch leaves its commits reachable only from the
    /// reflog, so this is what a confirmation has to say out loud.
    pub merged: bool,
}

impl Branch {
    /// A branch with nothing tracked and nothing known about its history.
    pub fn new(name: impl Into<String>, current: bool) -> Self {
        Self {
            name: name.into(),
            current,
            checkouts: Vec::new(),
            upstream: None,
            merged: false,
        }
    }
}

impl Head {
    /// A short label for a status surface.
    pub fn label(&self) -> String {
        match self {
            Self::Branch(name) => name.clone(),
            Self::Unborn(name) => format!("{name} (unborn)"),
            Self::Detached(commit) => {
                format!("@{}", &commit[..commit.len().min(7)])
            }
        }
    }
}

/// How far the current branch has drifted from its upstream.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Divergence {
    pub ahead: usize,
    pub behind: usize,
}

/// How strongly the person authorized a destructive Git operation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DeletionAuthorization {
    Enter,
    Typed,
}

/// A reviewed branch deletion tied to the exact ref tip it described.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BranchDeletionPlan {
    pub branch: String,
    pub tip: String,
    pub upstream: Option<Upstream>,
    /// Other local branches whose tips contain `tip`.
    pub retaining_branches: Vec<String>,
    pub required_authorization: DeletionAuthorization,
}

/// A reviewed worktree removal tied to the checkout identity it described.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WorktreeRemovalPlan {
    pub path: PathBuf,
    pub head: Option<String>,
    /// Short local branch name, when the worktree is attached.
    pub branch: Option<String>,
    pub upstream: Option<Upstream>,
    pub detached_retained: bool,
    pub required_authorization: DeletionAuthorization,
}

/// What happened to one side of one path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileState {
    Unmodified,
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Ignored,
    /// Both sides of an unfinished merge.
    Conflicted,
}

impl FileState {
    /// The one-letter form Git uses in its own short status.
    pub const fn marker(self) -> char {
        match self {
            Self::Unmodified => '.',
            Self::Added => 'A',
            Self::Modified => 'M',
            Self::Deleted => 'D',
            Self::Renamed => 'R',
            Self::Copied => 'C',
            Self::TypeChanged => 'T',
            Self::Untracked => '?',
            Self::Ignored => '!',
            Self::Conflicted => 'U',
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            b'.' => Self::Unmodified,
            b'A' => Self::Added,
            b'M' => Self::Modified,
            b'D' => Self::Deleted,
            b'R' => Self::Renamed,
            b'C' => Self::Copied,
            b'T' => Self::TypeChanged,
            _ => return None,
        })
    }
}

/// One path Git reports as differing from `HEAD`, from the index, or from both.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileStatus {
    /// Repository-relative, exactly as Git spelled it.
    pub path: PathBuf,
    /// Where a rename or copy came from.
    pub original_path: Option<PathBuf>,
    /// `HEAD` against the index.
    pub index: FileState,
    /// The index against the working tree.
    pub worktree: FileState,
}

impl FileStatus {
    pub const fn is_conflicted(&self) -> bool {
        matches!(self.index, FileState::Conflicted)
    }

    pub const fn is_untracked(&self) -> bool {
        matches!(self.worktree, FileState::Untracked)
    }

    /// Whether the index holds a change that a commit would take.
    pub const fn is_staged(&self) -> bool {
        !matches!(
            self.index,
            FileState::Unmodified | FileState::Untracked | FileState::Ignored
        )
    }
}

/// How many files are in each state, for a one-line summary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StatusCounts {
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
    pub untracked: usize,
    pub conflicted: usize,
}

impl StatusCounts {
    pub const fn is_empty(&self) -> bool {
        self.added == 0
            && self.modified == 0
            && self.deleted == 0
            && self.untracked == 0
            && self.conflicted == 0
    }
}

/// A point-in-time answer to "what has changed here".
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryStatus {
    pub head: Head,
    pub upstream: Option<String>,
    pub divergence: Divergence,
    pub files: Vec<FileStatus>,
}

impl RepositoryStatus {
    /// Counts each path once, under the most consequential thing that happened
    /// to it. A conflict outranks a deletion, which outranks an addition,
    /// which outranks a modification, so one file never inflates two numbers.
    pub fn counts(&self) -> StatusCounts {
        let mut counts = StatusCounts::default();
        for file in &self.files {
            if file.is_conflicted() {
                counts.conflicted += 1;
            } else if file.is_untracked() {
                counts.untracked += 1;
            } else if matches!(file.index, FileState::Deleted)
                || matches!(file.worktree, FileState::Deleted)
            {
                counts.deleted += 1;
            } else if matches!(file.index, FileState::Added) {
                counts.added += 1;
            } else {
                counts.modified += 1;
            }
        }
        counts
    }

    /// A compact status-surface label: the branch, then what is outstanding.
    pub fn summary(&self) -> String {
        let mut summary = self.head.label();
        let Divergence { ahead, behind } = self.divergence;
        if ahead > 0 {
            summary.push_str(&format!(" ↑{ahead}"));
        }
        if behind > 0 {
            summary.push_str(&format!(" ↓{behind}"));
        }
        let counts = self.counts();
        for (marker, count) in [
            ("+", counts.added),
            ("~", counts.modified),
            ("-", counts.deleted),
            ("?", counts.untracked),
            ("!", counts.conflicted),
        ] {
            if count > 0 {
                summary.push_str(&format!(" {marker}{count}"));
            }
        }
        summary
    }
}

/// What a path looked like the last time it was staged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BaseContent {
    /// Nothing unconflicted is recorded for the path: it is untracked, newly
    /// added and never staged, or in the middle of a merge. There is no base
    /// to compare against, so no line is "changed" rather than every line
    /// being.
    Absent,
    /// Staged, but not text Runyte can diff.
    Binary,
    Text(String),
}

/// The complete file versions on the two sides of one Git comparison.
///
/// `previous` is the index for an unstaged comparison and `HEAD` for a staged
/// one. `current` is respectively the working tree and the index. Reusing
/// [`BaseContent`] keeps absence and binary content explicit on either side.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileComparison {
    pub previous: BaseContent,
    pub current: BaseContent,
}

/// Which pair of trees a diff compares.
///
/// The two questions people actually ask are "what have I not staged yet" and
/// "what would a commit take", and they are different comparisons rather than
/// two views of one.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DiffScope {
    /// The index against the working tree.
    Unstaged,
    /// `HEAD` against the index: exactly what a commit would record.
    Staged,
}

/// The Git operations Runyte actually performs.
///
/// The trait is deliberately narrow. It grows an operation when a caller needs
/// one, not in anticipation: every method here is something the editor asks
/// for today, which is what keeps a fake implementation honest and a real one
/// small.
pub trait GitProvider {
    /// The repository owning `start`, or `None` when nothing does.
    fn discover(&self, start: &Path) -> Result<Option<Repository>>;

    /// Which files differ from `HEAD` or from the index, and where `HEAD` is.
    fn status(&self, repository: &Repository) -> Result<RepositoryStatus>;

    /// How many lines each of those files adds and removes, per side.
    ///
    /// Separate from [`GitProvider::status`] because it costs another read of
    /// both trees and only one view shows it, and it takes the status it
    /// describes because Git's counting stops at the index: an untracked file
    /// has nothing to be compared against, so a provider that wants to count
    /// one has to be told which files those are.
    fn status_stats(
        &self,
        _repository: &Repository,
        _status: &RepositoryStatus,
    ) -> Result<StatusStats> {
        Ok(StatusStats::default())
    }

    /// Current commit object, absent before the first commit. Services use it
    /// to distinguish a branch label that stayed put from files changed by an
    /// external commit or branch switch.
    fn head_oid(&self, _repository: &Repository) -> Result<Option<String>> {
        Ok(None)
    }

    /// Local branches, marking the one `HEAD` currently names when attached.
    fn branches(&self, repository: &Repository) -> Result<Vec<Branch>>;

    /// Every checkout registered with the repository's common Git directory.
    fn worktrees(&self, _repository: &Repository) -> Result<Vec<Worktree>> {
        Err(GitError::Unavailable {
            detail: "this Git provider does not expose worktrees".to_owned(),
        })
    }

    /// Creates a linked worktree at an explicit destination.
    fn create_worktree(&self, _repository: &Repository, _request: &WorktreeCreate) -> Result<()> {
        Err(GitError::Unavailable {
            detail: "this Git provider cannot create worktrees".to_owned(),
        })
    }

    /// Removes one ordinary linked worktree without forcing dirty content.
    ///
    /// Callers retain responsibility for refusing the current, locked, bare,
    /// and unavailable worktrees before this reaches Git. Git's
    /// own clean-worktree check remains the final authority immediately before
    /// removal, so a race cannot silently discard tracked or untracked files.
    fn remove_worktree(&self, _repository: &Repository, _path: &Path) -> Result<()> {
        Err(GitError::Unavailable {
            detail: "this Git provider cannot remove worktrees".to_owned(),
        })
    }

    /// Prepares a clean, identity-bound worktree removal for confirmation.
    fn prepare_worktree_removal(
        &self,
        repository: &Repository,
        path: &Path,
    ) -> Result<WorktreeRemovalPlan> {
        let worktree = self
            .worktrees(repository)?
            .into_iter()
            .find(|worktree| worktree.path == path)
            .ok_or_else(|| GitError::Failed {
                command: "git worktree list".to_owned(),
                code: None,
                stderr: format!("{} is no longer a registered worktree", path.display()),
            })?;
        let branch = worktree.branch.as_deref().map(|branch| {
            branch
                .strip_prefix("refs/heads/")
                .unwrap_or(branch)
                .to_owned()
        });
        let upstream = branch.as_deref().and_then(|name| {
            self.branches(repository)
                .ok()?
                .into_iter()
                .find(|candidate| candidate.name == name)?
                .upstream
        });
        let required_authorization = if upstream.as_ref().is_some_and(|upstream| {
            upstream
                .divergence
                .is_none_or(|divergence| divergence.ahead > 0)
        }) {
            DeletionAuthorization::Typed
        } else {
            DeletionAuthorization::Enter
        };
        Ok(WorktreeRemovalPlan {
            path: worktree.path,
            head: worktree.head,
            branch,
            upstream,
            detached_retained: false,
            required_authorization,
        })
    }

    /// Applies only the worktree removal that was reviewed, after rechecking it.
    fn remove_worktree_guarded(
        &self,
        repository: &Repository,
        plan: &WorktreeRemovalPlan,
        authorization: DeletionAuthorization,
    ) -> Result<()> {
        let current = self.prepare_worktree_removal(repository, &plan.path)?;
        if current != *plan {
            return Err(stale_deletion("worktree"));
        }
        if authorization < current.required_authorization {
            return Err(typed_deletion_required("worktree"));
        }
        self.remove_worktree(repository, &plan.path)
    }

    /// One bounded topological history page, continued by object identity.
    fn log_page(&self, _repository: &Repository, _request: &LogRequest) -> Result<LogPage> {
        Err(GitError::Unavailable {
            detail: "this Git provider does not expose history".to_owned(),
        })
    }

    /// A bounded set of commits reachable from `HEAD`, including full messages
    /// for native fuzzy ranking in the editor.
    fn search_commits(&self, _repository: &Repository) -> Result<CommitSearchResult> {
        Err(GitError::Unavailable {
            detail: "this Git provider does not expose searchable history".to_owned(),
        })
    }

    /// Whether a full commit identity remains reachable from the current HEAD.
    fn history_contains(&self, _repository: &Repository, _oid: &str) -> Result<bool> {
        Ok(false)
    }

    fn stashes(&self, _repository: &Repository) -> Result<Vec<StashEntry>> {
        Err(GitError::Unavailable {
            detail: "this Git provider does not expose stashes".to_owned(),
        })
    }

    fn mutate_stash(&self, _repository: &Repository, _mutation: &StashMutation) -> Result<String> {
        Err(GitError::Unavailable {
            detail: "this Git provider cannot change stashes".to_owned(),
        })
    }

    fn repository_fingerprint(&self, _repository: &Repository) -> Result<RepositoryFingerprint> {
        Err(GitError::Unavailable {
            detail: "this Git provider does not expose revision fingerprints".to_owned(),
        })
    }

    fn apply_partial(
        &self,
        _repository: &Repository,
        _request: &PartialStageRequest,
    ) -> Result<()> {
        Err(GitError::Unavailable {
            detail: "this Git provider cannot apply partial patches".to_owned(),
        })
    }

    fn prepare_partial(
        &self,
        _repository: &Repository,
        _selection: &PartialStageSelection,
    ) -> Result<PartialStageRequest> {
        Err(GitError::Unavailable {
            detail: "this Git provider cannot prepare partial patches".to_owned(),
        })
    }

    /// Bounded metadata and patch for one full object identity.
    fn commit_detail(&self, _repository: &Repository, _oid: &str) -> Result<CommitDetail> {
        Err(GitError::Unavailable {
            detail: "this Git provider does not expose commit details".to_owned(),
        })
    }

    /// Machine-readable attribution for the supplied live buffer text.
    fn blame(&self, _repository: &Repository, _request: &BlameRequest) -> Result<Vec<BlameLine>> {
        Err(GitError::Unavailable {
            detail: "this Git provider does not expose blame".to_owned(),
        })
    }

    /// Switches to one local branch, refusing whenever tracked or untracked
    /// work differs from `HEAD`.
    fn checkout_branch(&self, repository: &Repository, branch: &str) -> Result<()>;

    /// Creates `branch` at `start_point` and switches to it.
    ///
    /// One operation rather than two because a branch created and then not
    /// switched to is a state nobody asked for. It refuses a dirty working tree
    /// exactly as [`GitProvider::checkout_branch`] does, and it refuses before
    /// creating anything, so a refusal leaves the repository untouched.
    fn create_branch(&self, repository: &Repository, branch: &str, start_point: &str)
    -> Result<()>;

    /// Removes one local branch.
    ///
    /// `force` deletes a branch whose commits are not reachable from `HEAD`.
    /// Unlike [`GitProvider::discard`], what this drops is still in the object
    /// database and named by the reflog, so it is recoverable for as long as
    /// the reflog keeps it.
    fn delete_branch(&self, repository: &Repository, branch: &str, force: bool) -> Result<()>;

    /// Prepares a branch deletion and records which refs retain its tip.
    fn prepare_branch_deletion(
        &self,
        repository: &Repository,
        branch: &str,
    ) -> Result<BranchDeletionPlan> {
        let branches = self.branches(repository)?;
        let target = branches
            .iter()
            .find(|candidate| candidate.name == branch)
            .ok_or_else(|| GitError::Failed {
                command: "git branch".to_owned(),
                code: None,
                stderr: format!("`{branch}` is not a local branch"),
            })?;
        let retaining_branches = target
            .merged
            .then(|| {
                branches
                    .iter()
                    .find(|candidate| candidate.current)
                    .map(|candidate| candidate.name.clone())
            })
            .flatten()
            .into_iter()
            .collect::<Vec<_>>();
        let upstream_retains = target.upstream.as_ref().is_some_and(|upstream| {
            upstream
                .divergence
                .is_some_and(|divergence| divergence.ahead == 0)
        });
        Ok(BranchDeletionPlan {
            branch: branch.to_owned(),
            tip: branch.to_owned(),
            upstream: target.upstream.clone(),
            required_authorization: if upstream_retains || !retaining_branches.is_empty() {
                DeletionAuthorization::Enter
            } else {
                DeletionAuthorization::Typed
            },
            retaining_branches,
        })
    }

    /// Prepares a branch deletion that a cascade is about to reach through the
    /// one checkout it is removing on the way.
    ///
    /// The ordinary review refuses a branch checked out anywhere, because Git
    /// refuses to delete one. A cascade removes that checkout first, so by the
    /// time anything is deleted the branch is not checked out; what it needs
    /// here is the review, alongside the removal rather than after it. Only the
    /// named checkout is tolerated — any other is still a reason to refuse.
    ///
    /// The default is the ordinary review, which is already correct for a
    /// provider whose review carries no checkout guard.
    fn prepare_branch_deletion_through(
        &self,
        repository: &Repository,
        branch: &str,
        _checkout: &Path,
    ) -> Result<BranchDeletionPlan> {
        self.prepare_branch_deletion(repository, branch)
    }

    /// Deletes only the exact branch tip that was reviewed.
    fn delete_branch_guarded(
        &self,
        repository: &Repository,
        plan: &BranchDeletionPlan,
        authorization: DeletionAuthorization,
    ) -> Result<()> {
        let current = self.prepare_branch_deletion(repository, &plan.branch)?;
        if current != *plan {
            return Err(stale_deletion("branch"));
        }
        if authorization < current.required_authorization {
            return Err(typed_deletion_required("branch"));
        }
        self.delete_branch(repository, &plan.branch, true)
    }

    /// The staged text of one path.
    ///
    /// The index, not `HEAD`, is the base: staging a change should make its
    /// gutter marks go away, because from the working tree's point of view
    /// there is nothing left to notice about that line.
    fn staged_content(&self, repository: &Repository, path: &Path) -> Result<BaseContent>;

    /// A unified diff, for one path or for the whole working tree.
    ///
    /// This is Git's own patch text rather than anything Runyte derives. The
    /// gutter is the live view of a buffer being edited; a diff is the record
    /// of what is on disk, and the two are deliberately produced by different
    /// means because they answer different questions.
    fn diff(
        &self,
        repository: &Repository,
        scope: DiffScope,
        path: Option<&Path>,
    ) -> Result<String>;

    /// The complete contents on both sides of a per-file diff.
    fn file_comparison(
        &self,
        repository: &Repository,
        scope: DiffScope,
        path: &Path,
    ) -> Result<FileComparison>;

    /// Records the working-tree state of one path in the index.
    fn stage(&self, repository: &Repository, path: &Path) -> Result<()>;

    /// Returns one path in the index to what `HEAD` has for it, leaving the
    /// working tree alone.
    fn unstage(&self, repository: &Repository, path: &Path) -> Result<()>;

    /// Throws a path's uncommitted changes away, staged and unstaged alike,
    /// restoring it to what `HEAD` holds.
    ///
    /// This is the one operation here with nothing behind it: the discarded
    /// content was never an object, so no reflog and no `fsck` will find it
    /// again. Callers are expected to have asked first.
    fn discard(&self, repository: &Repository, path: &Path) -> Result<()>;

    /// Fetches the current branch's upstream and fast-forwards onto it.
    ///
    /// Fast-forward only. A pull that had to merge could leave the working tree
    /// mid-conflict, and Runyte has no surface for finishing one; refusing is
    /// an outcome the reader can act on, where a conflicted tree they cannot
    /// resolve here is not.
    ///
    /// This reaches the network, so unlike everything else on this trait it can
    /// take as long as a remote takes to answer, and it carries a deadline
    /// rather than waiting forever. Returns what Git said about the merge.
    ///
    /// A branch that has drifted both ways fails with [`GitError::Diverged`]
    /// rather than with whatever `--ff-only` printed, because that one refusal
    /// has a next step and [`GitProvider::rebase_onto_upstream`] is it.
    fn pull(&self, repository: &Repository) -> Result<String>;

    /// Replays the current branch's unpushed commits on top of its upstream,
    /// which is the one way out of the divergence [`GitProvider::pull`]
    /// refuses.
    ///
    /// Rebase rather than merge: the fast-forward-only pull above already says
    /// this history is meant to stay linear, and a rebase needs no commit
    /// message, which Runyte would otherwise have to invent or ask for.
    ///
    /// The invariant `pull` protects holds here too. A rebase that stops on a
    /// conflict leaves the working tree mid-replay with no surface in Runyte
    /// to finish it, so an implementation that cannot complete must undo its
    /// own attempt and report, rather than hand back a repository the reader
    /// cannot get out of.
    ///
    /// Reaches the network on the same terms as [`GitProvider::pull`].
    fn rebase_onto_upstream(&self, repository: &Repository) -> Result<String>;

    /// Publishes one local branch to the upstream it tracks.
    ///
    /// A branch tracking nothing is published to the repository's remote and
    /// set to track it, which is the only sense "push this branch" can have the
    /// first time. Nothing here forces: a rejected non-fast-forward is a
    /// refusal to report rather than something to overrule.
    ///
    /// Reaches the network on the same terms as [`GitProvider::pull`]. Returns
    /// what Git said about the push.
    fn push(&self, repository: &Repository, branch: &str) -> Result<String>;

    /// Records everything in the index as a commit.
    ///
    /// The index and nothing else: what is committed is what a reader has
    /// already been shown under "Staged", so there is one meaning of the word
    /// rather than one per place it was invoked from.
    ///
    /// Returns what Git said about the commit it made. This runs the
    /// repository's hooks, which are arbitrary programs of unbounded duration.
    fn commit(&self, repository: &Repository, message: &str) -> Result<String>;
}

/// A provider that answers from memory, for tests that are about the editor
/// rather than about Git.
///
/// It also counts the calls it receives, which is how the tests assert the
/// thing that matters most about the caching above: that editing a buffer does
/// not run Git.
#[cfg(test)]
#[derive(Debug)]
pub struct MemoryGitProvider {
    repository: Repository,
    /// What `HEAD` holds, for staged side-by-side comparisons.
    head: std::collections::HashMap<PathBuf, BaseContent>,
    /// What the index holds, which `stage` and `unstage` move in and out of.
    staged: std::cell::RefCell<std::collections::HashMap<PathBuf, BaseContent>>,
    /// What the working tree holds, which is what staging records.
    working: std::collections::HashMap<PathBuf, String>,
    /// Every message committed through this provider, in order.
    committed: std::cell::RefCell<Vec<String>>,
    /// Every path whose changes were thrown away, in order.
    discarded: std::cell::RefCell<Vec<PathBuf>>,
    branches: std::cell::RefCell<Vec<Branch>>,
    checked_out: std::cell::RefCell<Vec<String>>,
    /// Every branch created through this provider, with the start point asked
    /// for, in order.
    created: std::cell::RefCell<Vec<(String, String)>>,
    /// Every branch deleted through this provider, and whether it was forced.
    deleted: std::cell::RefCell<Vec<(String, bool)>>,
    worktrees: std::cell::RefCell<Vec<Worktree>>,
    removed_worktrees: std::cell::RefCell<Vec<PathBuf>>,
    /// How many times the upstream was pulled, how many times the current
    /// branch was replayed onto it, and every branch pushed.
    pulled: std::cell::Cell<usize>,
    rebased: std::cell::Cell<usize>,
    pushed: std::cell::RefCell<Vec<String>>,
    /// Refuses only network operations, so a test can reach one with
    /// everything around it working.
    refuse_network: bool,
    status: std::cell::RefCell<RepositoryStatus>,
    /// What `status_stats` answers with, empty unless a test set it.
    stats: StatusStats,
    /// Refuses only line counts, so snapshots can prove those decorations are
    /// optional without making the status itself fail.
    refuse_stats: bool,
    diff: String,
    failing: bool,
    /// Refuses only commits, so a test can reach the commit path with
    /// everything before it working.
    refuse_commits: bool,
    calls: std::cell::Cell<usize>,
}

#[cfg(test)]
impl MemoryGitProvider {
    pub fn new(repository: Repository) -> Self {
        Self {
            repository,
            head: std::collections::HashMap::new(),
            staged: std::cell::RefCell::new(std::collections::HashMap::new()),
            working: std::collections::HashMap::new(),
            committed: std::cell::RefCell::new(Vec::new()),
            discarded: std::cell::RefCell::new(Vec::new()),
            branches: std::cell::RefCell::new(vec![Branch::new("main", true)]),
            checked_out: std::cell::RefCell::new(Vec::new()),
            created: std::cell::RefCell::new(Vec::new()),
            deleted: std::cell::RefCell::new(Vec::new()),
            worktrees: std::cell::RefCell::new(Vec::new()),
            removed_worktrees: std::cell::RefCell::new(Vec::new()),
            pulled: std::cell::Cell::new(0),
            rebased: std::cell::Cell::new(0),
            pushed: std::cell::RefCell::new(Vec::new()),
            refuse_network: false,
            status: std::cell::RefCell::new(RepositoryStatus {
                head: Head::Branch("main".to_owned()),
                upstream: None,
                divergence: Divergence::default(),
                files: Vec::new(),
            }),
            stats: StatusStats::default(),
            refuse_stats: false,
            diff: String::new(),
            failing: false,
            refuse_commits: false,
            calls: std::cell::Cell::new(0),
        }
    }

    #[must_use]
    pub fn with_staged(mut self, relative: &str, text: &str) -> Self {
        self.set_staged(relative, text);
        self
    }

    #[must_use]
    pub fn with_head(mut self, relative: &str, text: &str) -> Self {
        self.head
            .insert(PathBuf::from(relative), BaseContent::Text(text.to_owned()));
        self
    }

    #[must_use]
    pub fn with_binary(mut self, relative: &str) -> Self {
        self.staged
            .get_mut()
            .insert(PathBuf::from(relative), BaseContent::Binary);
        self
    }

    /// What the working tree holds for a path, which is what `stage` records.
    #[must_use]
    pub fn with_working(mut self, relative: &str, text: &str) -> Self {
        self.working
            .insert(PathBuf::from(relative), text.to_owned());
        self
    }

    #[must_use]
    pub fn with_diff(mut self, diff: &str) -> Self {
        self.diff = diff.to_owned();
        self
    }

    #[must_use]
    pub fn with_status(mut self, status: RepositoryStatus) -> Self {
        self.status = std::cell::RefCell::new(status);
        self
    }

    /// Records what one path's change costs in lines, on one side.
    #[must_use]
    pub fn with_line_stats(mut self, scope: DiffScope, relative: &str, stats: LineStats) -> Self {
        self.stats.insert(scope, PathBuf::from(relative), stats);
        self
    }

    #[must_use]
    pub fn refusing_stats(mut self) -> Self {
        self.refuse_stats = true;
        self
    }

    #[must_use]
    pub fn with_branches(mut self, names: &[&str], current: &str) -> Self {
        self.branches = std::cell::RefCell::new(
            names
                .iter()
                .map(|name| Branch::new(*name, *name == current))
                .collect(),
        );
        self.status.get_mut().head = Head::Branch(current.to_owned());
        self
    }

    /// Replaces one branch's tracking and merge state, for a test about how
    /// either is reported rather than about Git.
    #[must_use]
    pub fn with_branch_detail(self, name: &str, upstream: Option<Upstream>, merged: bool) -> Self {
        for branch in self.branches.borrow_mut().iter_mut() {
            if branch.name == name {
                branch.upstream = upstream.clone();
                branch.merged = merged;
            }
        }
        self
    }

    #[must_use]
    pub fn with_branch_checkout(self, name: &str, path: PathBuf) -> Self {
        if let Some(branch) = self
            .branches
            .borrow_mut()
            .iter_mut()
            .find(|candidate| candidate.name == name)
        {
            branch.checkouts.push(path);
        }
        self
    }

    #[must_use]
    pub fn with_worktrees(mut self, worktrees: Vec<Worktree>) -> Self {
        self.worktrees = std::cell::RefCell::new(worktrees);
        self
    }

    #[must_use]
    pub fn failing(mut self) -> Self {
        self.failing = true;
        self
    }

    /// Refuses commits alone, the way a rejecting hook or an unset identity
    /// does: everything up to the commit works.
    #[must_use]
    pub fn refusing_commits(mut self) -> Self {
        self.refuse_commits = true;
        self
    }

    /// Puts a text in the index directly, as a commit or an earlier staging
    /// would have. Named apart from the trait's `stage`, which records what
    /// the working tree holds rather than what a test says.
    pub fn set_staged(&mut self, relative: &str, text: &str) {
        self.staged
            .get_mut()
            .insert(PathBuf::from(relative), BaseContent::Text(text.to_owned()));
    }

    /// The messages committed so far, which is what a test asserts on.
    pub fn commits(&self) -> Vec<String> {
        self.committed.borrow().clone()
    }

    /// The paths discarded so far, which is what a test asserts on.
    pub fn discards(&self) -> Vec<PathBuf> {
        self.discarded.borrow().clone()
    }

    pub fn checkouts(&self) -> Vec<String> {
        self.checked_out.borrow().clone()
    }

    /// The branches created so far, each with the start point asked for.
    pub fn creations(&self) -> Vec<(String, String)> {
        self.created.borrow().clone()
    }

    /// The branches deleted so far, each with whether the delete was forced.
    pub fn deletions(&self) -> Vec<(String, bool)> {
        self.deleted.borrow().clone()
    }

    pub fn removed_worktrees(&self) -> Vec<PathBuf> {
        self.removed_worktrees.borrow().clone()
    }

    /// How many pulls have been made, and which branches were pushed.
    pub fn pulls(&self) -> usize {
        self.pulled.get()
    }

    pub fn pushes(&self) -> Vec<String> {
        self.pushed.borrow().clone()
    }

    /// How many times the current branch was replayed onto its upstream.
    pub fn rebases(&self) -> usize {
        self.rebased.get()
    }

    /// The refusal a pull owes a current branch that has drifted both ways,
    /// or `None` when a fast-forward is still possible.
    fn divergence_refusal(&self) -> Option<GitError> {
        let branches = self.branches.borrow();
        let branch = branches.iter().find(|branch| branch.current)?;
        let upstream = branch.upstream.as_ref()?;
        let divergence = upstream.divergence?;
        (divergence.ahead > 0 && divergence.behind > 0).then(|| GitError::Diverged {
            branch: branch.name.clone(),
            upstream: upstream.name.clone(),
            ahead: divergence.ahead,
            behind: divergence.behind,
        })
    }

    /// Refuses pull and push alone, the way an unreachable remote does:
    /// everything that does not leave the machine still works.
    #[must_use]
    pub fn refusing_network(mut self) -> Self {
        self.refuse_network = true;
        self
    }

    pub fn calls(&self) -> usize {
        self.calls.get()
    }

    /// Moves a file's change from one side of the index to the other, so a
    /// test can watch a round trip rather than a frozen answer.
    fn move_side(&self, relative: &Path, stage: bool) {
        let mut status = self.status.borrow_mut();
        let Some(file) = status.files.iter_mut().find(|file| file.path == relative) else {
            return;
        };
        if stage {
            file.index = match file.worktree {
                FileState::Untracked => FileState::Added,
                FileState::Unmodified => file.index,
                worktree => worktree,
            };
            file.worktree = FileState::Unmodified;
        } else {
            file.worktree = match file.index {
                FileState::Added => FileState::Untracked,
                FileState::Unmodified => file.worktree,
                index => index,
            };
            file.index = FileState::Unmodified;
        }
    }

    fn refuse<T>(&self) -> Result<T> {
        Err(GitError::Failed {
            command: "git".to_owned(),
            code: Some(128),
            stderr: "fatal: test provider refuses".to_owned(),
        })
    }
}

#[cfg(test)]
impl GitProvider for MemoryGitProvider {
    fn discover(&self, start: &Path) -> Result<Option<Repository>> {
        if self.failing {
            return self.refuse();
        }
        Ok(self
            .repository
            .contains(start)
            .then(|| self.repository.clone()))
    }

    fn status(&self, _repository: &Repository) -> Result<RepositoryStatus> {
        if self.failing {
            return self.refuse();
        }
        Ok(self.status.borrow().clone())
    }

    fn status_stats(
        &self,
        _repository: &Repository,
        _status: &RepositoryStatus,
    ) -> Result<StatusStats> {
        if self.failing || self.refuse_stats {
            return self.refuse();
        }
        Ok(self.stats.clone())
    }

    fn branches(&self, _repository: &Repository) -> Result<Vec<Branch>> {
        if self.failing {
            return self.refuse();
        }
        Ok(self.branches.borrow().clone())
    }

    fn worktrees(&self, _repository: &Repository) -> Result<Vec<Worktree>> {
        if self.failing {
            return self.refuse();
        }
        Ok(self.worktrees.borrow().clone())
    }

    fn remove_worktree(&self, _repository: &Repository, path: &Path) -> Result<()> {
        if self.failing {
            return self.refuse();
        }
        let mut worktrees = self.worktrees.borrow_mut();
        let Some(position) = worktrees.iter().position(|worktree| worktree.path == path) else {
            return self.refuse();
        };
        let removed = worktrees.remove(position);
        if let Some(branch) = removed.branch.as_deref() {
            let branch = branch.strip_prefix("refs/heads/").unwrap_or(branch);
            if let Some(branch) = self
                .branches
                .borrow_mut()
                .iter_mut()
                .find(|candidate| candidate.name == branch)
            {
                branch.checkouts.retain(|checkout| checkout != path);
            }
        }
        self.removed_worktrees.borrow_mut().push(path.to_path_buf());
        Ok(())
    }

    fn checkout_branch(&self, _repository: &Repository, branch: &str) -> Result<()> {
        if self.failing {
            return self.refuse();
        }
        let files = self.status.borrow().files.len();
        if files > 0 {
            return Err(GitError::DirtyWorktree { files });
        }
        let mut branches = self.branches.borrow_mut();
        if !branches.iter().any(|candidate| candidate.name == branch) {
            return self.refuse();
        }
        for candidate in branches.iter_mut() {
            candidate.current = candidate.name == branch;
        }
        self.status.borrow_mut().head = Head::Branch(branch.to_owned());
        self.checked_out.borrow_mut().push(branch.to_owned());
        Ok(())
    }

    fn create_branch(
        &self,
        _repository: &Repository,
        branch: &str,
        start_point: &str,
    ) -> Result<()> {
        if self.failing {
            return self.refuse();
        }
        let files = self.status.borrow().files.len();
        if files > 0 {
            return Err(GitError::DirtyWorktree { files });
        }
        let mut branches = self.branches.borrow_mut();
        if branches.iter().any(|candidate| candidate.name == branch) {
            return self.refuse();
        }
        for candidate in branches.iter_mut() {
            candidate.current = false;
        }
        // A branch created at the row someone was pointing at holds exactly the
        // commits that row held, so it starts out reachable from the new `HEAD`.
        branches.push(Branch {
            name: branch.to_owned(),
            current: true,
            checkouts: Vec::new(),
            upstream: None,
            merged: true,
        });
        branches.sort_by(|left, right| left.name.cmp(&right.name));
        self.status.borrow_mut().head = Head::Branch(branch.to_owned());
        self.created
            .borrow_mut()
            .push((branch.to_owned(), start_point.to_owned()));
        self.checked_out.borrow_mut().push(branch.to_owned());
        Ok(())
    }

    fn delete_branch(&self, _repository: &Repository, branch: &str, force: bool) -> Result<()> {
        if self.failing {
            return self.refuse();
        }
        let mut branches = self.branches.borrow_mut();
        let Some(position) = branches
            .iter()
            .position(|candidate| candidate.name == branch)
        else {
            return self.refuse();
        };
        if branches[position].current {
            return self.refuse();
        }
        if !branches[position].merged && !force {
            return self.refuse();
        }
        branches.remove(position);
        self.deleted.borrow_mut().push((branch.to_owned(), force));
        Ok(())
    }

    fn staged_content(&self, repository: &Repository, path: &Path) -> Result<BaseContent> {
        self.calls.set(self.calls.get() + 1);
        if self.failing {
            return self.refuse();
        }
        let relative = repository
            .relative(path)
            .ok_or_else(|| GitError::NotARepository {
                path: path.to_path_buf(),
            })?;
        Ok(self
            .staged
            .borrow()
            .get(relative)
            .cloned()
            .unwrap_or(BaseContent::Absent))
    }

    fn diff(
        &self,
        _repository: &Repository,
        _scope: DiffScope,
        _path: Option<&Path>,
    ) -> Result<String> {
        if self.failing {
            return self.refuse();
        }
        Ok(self.diff.clone())
    }

    fn file_comparison(
        &self,
        repository: &Repository,
        scope: DiffScope,
        path: &Path,
    ) -> Result<FileComparison> {
        if self.failing {
            return self.refuse();
        }
        let relative = repository
            .relative(path)
            .ok_or_else(|| GitError::NotARepository {
                path: path.to_path_buf(),
            })?;
        let staged = self
            .staged
            .borrow()
            .get(relative)
            .cloned()
            .unwrap_or(BaseContent::Absent);
        Ok(match scope {
            DiffScope::Staged => FileComparison {
                previous: self
                    .head
                    .get(relative)
                    .cloned()
                    .unwrap_or(BaseContent::Absent),
                current: staged,
            },
            DiffScope::Unstaged => FileComparison {
                previous: staged,
                current: self
                    .working
                    .get(relative)
                    .cloned()
                    .map(BaseContent::Text)
                    .unwrap_or(BaseContent::Absent),
            },
        })
    }

    fn stage(&self, repository: &Repository, path: &Path) -> Result<()> {
        if self.failing {
            return self.refuse();
        }
        let relative = repository
            .relative(path)
            .ok_or_else(|| GitError::NotARepository {
                path: path.to_path_buf(),
            })?;
        match self.working.get(relative) {
            Some(text) => {
                self.staged
                    .borrow_mut()
                    .insert(relative.to_path_buf(), BaseContent::Text(text.clone()));
            }
            None => {
                self.staged.borrow_mut().remove(relative);
            }
        }
        self.move_side(relative, true);
        Ok(())
    }

    fn discard(&self, repository: &Repository, path: &Path) -> Result<()> {
        if self.failing {
            return self.refuse();
        }
        let relative = repository
            .relative(path)
            .ok_or_else(|| GitError::NotARepository {
                path: path.to_path_buf(),
            })?;
        self.staged.borrow_mut().remove(relative);
        self.status
            .borrow_mut()
            .files
            .retain(|file| file.path != relative);
        self.discarded.borrow_mut().push(relative.to_path_buf());
        Ok(())
    }

    fn pull(&self, _repository: &Repository) -> Result<String> {
        if self.failing || self.refuse_network {
            return self.refuse();
        }
        // A branch that has drifted both ways has no fast-forward, so it gets
        // the refusal the real provider gives rather than a pull that quietly
        // succeeds only in memory.
        if let Some(diverged) = self.divergence_refusal() {
            return Err(diverged);
        }
        self.pulled.set(self.pulled.get() + 1);
        // Fast-forwarded onto the upstream, so the branch is level with it and
        // nothing is left behind to pull again.
        let mut branches = self.branches.borrow_mut();
        if let Some(branch) = branches.iter_mut().find(|branch| branch.current)
            && let Some(upstream) = branch.upstream.as_mut()
            && let Some(divergence) = upstream.divergence.as_mut()
        {
            divergence.behind = 0;
        }
        Ok("Fast-forward".to_owned())
    }

    fn rebase_onto_upstream(&self, _repository: &Repository) -> Result<String> {
        if self.failing || self.refuse_network {
            return self.refuse();
        }
        self.rebased.set(self.rebased.get() + 1);
        // Replayed onto the upstream: the local commits are still ahead of it,
        // and nothing is behind any more.
        let mut branches = self.branches.borrow_mut();
        if let Some(branch) = branches.iter_mut().find(|branch| branch.current)
            && let Some(upstream) = branch.upstream.as_mut()
            && let Some(divergence) = upstream.divergence.as_mut()
        {
            divergence.behind = 0;
        }
        Ok("Successfully rebased and updated refs/heads/main.".to_owned())
    }

    fn push(&self, _repository: &Repository, branch: &str) -> Result<String> {
        if self.failing || self.refuse_network {
            return self.refuse();
        }
        let mut branches = self.branches.borrow_mut();
        let Some(target) = branches
            .iter_mut()
            .find(|candidate| candidate.name == branch)
        else {
            return self.refuse();
        };
        // Published, so the upstream now holds everything the local branch did.
        match target.upstream.as_mut() {
            Some(upstream) => {
                if let Some(divergence) = upstream.divergence.as_mut() {
                    divergence.ahead = 0;
                }
            }
            None => {
                target.upstream = Some(Upstream::origin(branch, Some(Divergence::default())));
            }
        }
        self.pushed.borrow_mut().push(branch.to_owned());
        Ok(format!("To origin\n   abc..def  {branch} -> {branch}"))
    }

    fn commit(&self, _repository: &Repository, message: &str) -> Result<String> {
        if self.failing || self.refuse_commits {
            return self.refuse();
        }
        self.committed.borrow_mut().push(message.to_owned());
        // Everything the index held is now in a commit, so nothing is staged.
        let mut status = self.status.borrow_mut();
        for file in &mut status.files {
            file.index = FileState::Unmodified;
        }
        status
            .files
            .retain(|file| !matches!(file.worktree, FileState::Unmodified));
        Ok(format!(
            "[main 0000000] {}",
            message.lines().next().unwrap_or("")
        ))
    }

    fn unstage(&self, repository: &Repository, path: &Path) -> Result<()> {
        if self.failing {
            return self.refuse();
        }
        let relative = repository
            .relative(path)
            .ok_or_else(|| GitError::NotARepository {
                path: path.to_path_buf(),
            })?;
        self.staged.borrow_mut().remove(relative);
        self.move_side(relative, false);
        Ok(())
    }
}

/// Shared ownership of a fake, so a test can keep asking it what it was told
/// after handing it to the editor.
///
/// `Rc` rather than `Arc`: the fake keeps its state in cells, which is right
/// for a single-threaded test double and wrong for anything shared between
/// threads, so the pointer type says so.
#[cfg(test)]
impl GitProvider for std::rc::Rc<MemoryGitProvider> {
    fn discover(&self, start: &Path) -> Result<Option<Repository>> {
        self.as_ref().discover(start)
    }

    fn status(&self, repository: &Repository) -> Result<RepositoryStatus> {
        self.as_ref().status(repository)
    }

    fn branches(&self, repository: &Repository) -> Result<Vec<Branch>> {
        self.as_ref().branches(repository)
    }

    fn worktrees(&self, repository: &Repository) -> Result<Vec<Worktree>> {
        self.as_ref().worktrees(repository)
    }

    fn prepare_branch_deletion_through(
        &self,
        repository: &Repository,
        branch: &str,
        checkout: &Path,
    ) -> Result<BranchDeletionPlan> {
        self.as_ref()
            .prepare_branch_deletion_through(repository, branch, checkout)
    }

    fn remove_worktree(&self, repository: &Repository, path: &Path) -> Result<()> {
        self.as_ref().remove_worktree(repository, path)
    }

    fn checkout_branch(&self, repository: &Repository, branch: &str) -> Result<()> {
        self.as_ref().checkout_branch(repository, branch)
    }

    fn create_branch(
        &self,
        repository: &Repository,
        branch: &str,
        start_point: &str,
    ) -> Result<()> {
        self.as_ref().create_branch(repository, branch, start_point)
    }

    fn delete_branch(&self, repository: &Repository, branch: &str, force: bool) -> Result<()> {
        self.as_ref().delete_branch(repository, branch, force)
    }

    fn staged_content(&self, repository: &Repository, path: &Path) -> Result<BaseContent> {
        self.as_ref().staged_content(repository, path)
    }

    fn diff(
        &self,
        repository: &Repository,
        scope: DiffScope,
        path: Option<&Path>,
    ) -> Result<String> {
        self.as_ref().diff(repository, scope, path)
    }

    fn file_comparison(
        &self,
        repository: &Repository,
        scope: DiffScope,
        path: &Path,
    ) -> Result<FileComparison> {
        self.as_ref().file_comparison(repository, scope, path)
    }

    fn stage(&self, repository: &Repository, path: &Path) -> Result<()> {
        self.as_ref().stage(repository, path)
    }

    fn unstage(&self, repository: &Repository, path: &Path) -> Result<()> {
        self.as_ref().unstage(repository, path)
    }

    fn commit(&self, repository: &Repository, message: &str) -> Result<String> {
        self.as_ref().commit(repository, message)
    }

    fn pull(&self, repository: &Repository) -> Result<String> {
        self.as_ref().pull(repository)
    }

    fn rebase_onto_upstream(&self, repository: &Repository) -> Result<String> {
        self.as_ref().rebase_onto_upstream(repository)
    }

    fn push(&self, repository: &Repository, branch: &str) -> Result<String> {
        self.as_ref().push(repository, branch)
    }

    fn discard(&self, repository: &Repository, path: &Path) -> Result<()> {
        self.as_ref().discard(repository, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Display` quotes the argument vector and Git's stderr, which is right
    /// for the interaction line and wrong for a durable file: a failing commit
    /// carries the message that was just typed, and stderr is unrestricted
    /// subprocess output.
    #[test]
    fn a_redacted_failure_keeps_no_argument_vector_or_subprocess_output() {
        let error = GitError::Failed {
            command: "git commit --cleanup=whitespace -m Refactor the SECRET parser".to_owned(),
            code: Some(128),
            stderr: "fatal: SECRET-FROM-STDERR".to_owned(),
        };

        let redacted = error.redacted();
        assert_eq!(redacted, "Git refused with status 128");
        assert!(error.to_string().contains("SECRET"), "the premise holds");
        for leaked in ["SECRET", "commit", "--cleanup", "fatal"] {
            assert!(!redacted.contains(leaked), "{leaked} survived: {redacted}");
        }

        for error in [
            GitError::Failed {
                command: "git push origin SECRET-BRANCH".to_owned(),
                code: None,
                stderr: "SECRET".to_owned(),
            },
            GitError::TooLarge {
                command: "git log SECRET".to_owned(),
                limit: 64,
            },
            GitError::TimedOut {
                command: "git fetch SECRET".to_owned(),
                seconds: 5,
            },
            GitError::Cancelled {
                command: "git merge SECRET".to_owned(),
            },
            GitError::Malformed {
                command: "git status SECRET".to_owned(),
                detail: "SECRET".to_owned(),
            },
            GitError::Unavailable {
                detail: "SECRET".to_owned(),
            },
            GitError::NotARepository {
                path: PathBuf::from("/home/someone/SECRET"),
            },
            GitError::Io {
                action: "read",
                path: PathBuf::from("/home/someone/SECRET"),
                detail: "SECRET".to_owned(),
            },
        ] {
            let redacted = error.redacted();
            assert!(
                !redacted.contains("SECRET"),
                "{error:?} leaked through {redacted}"
            );
            assert!(!redacted.is_empty());
        }
    }

    fn status_of(files: Vec<FileStatus>) -> RepositoryStatus {
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

    #[test]
    fn a_file_is_counted_once_under_its_most_consequential_state() {
        let status = status_of(vec![
            // Staged and then modified again: still one modified file.
            file("edited.rs", FileState::Modified, FileState::Modified),
            file("new.rs", FileState::Added, FileState::Unmodified),
            file("gone.rs", FileState::Deleted, FileState::Unmodified),
            file("stray.rs", FileState::Untracked, FileState::Untracked),
            file("clash.rs", FileState::Conflicted, FileState::Conflicted),
        ]);

        assert_eq!(
            status.counts(),
            StatusCounts {
                added: 1,
                modified: 1,
                deleted: 1,
                untracked: 1,
                conflicted: 1,
            }
        );
        assert_eq!(status.summary(), "main +1 ~1 -1 ?1 !1");
    }

    #[test]
    fn a_clean_repository_summarizes_to_its_branch_alone() {
        assert_eq!(status_of(Vec::new()).summary(), "main");
        assert!(status_of(Vec::new()).counts().is_empty());
    }

    #[test]
    fn divergence_and_detached_heads_read_as_labels() {
        let mut status = status_of(Vec::new());
        status.divergence = Divergence {
            ahead: 2,
            behind: 1,
        };
        assert_eq!(status.summary(), "main ↑2 ↓1");

        status.head = Head::Detached("1a2b3c4d5e6f".to_owned());
        assert_eq!(status.head.label(), "@1a2b3c4");
        status.head = Head::Unborn("main".to_owned());
        assert_eq!(status.head.label(), "main (unborn)");
    }

    #[test]
    fn a_repository_only_claims_paths_beneath_it() {
        let repository = Repository::new("/projects/runyte");

        assert_eq!(
            repository.relative(Path::new("/projects/runyte/src/git/mod.rs")),
            Some(Path::new("src/git/mod.rs"))
        );
        assert!(!repository.contains(Path::new("/projects/other/src/main.rs")));
        assert!(!repository.contains(Path::new("/projects/runyte-sibling/x")));
    }
}
