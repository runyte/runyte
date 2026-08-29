// SPDX-License-Identifier: MPL-2.0

//! Bounded asynchronous scheduling for editor-facing Git work.
//!
//! The scheduler serializes all work for one repository (reads included, so a
//! read cannot pass an earlier mutation), while independent repositories use
//! up to four workers. Workers return owned events and never touch editor
//! state. Duplicate queued or running reads share one subprocess result.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, channel, sync_channel},
    },
    time::Instant,
};

use tokio::sync::mpsc;

use super::{
    BaseContent, BlameLine, BlameRequest, Branch, BranchDeletionPlan, CommitDetail,
    CommitSearchResult, DeletionAuthorization, DiffScope, FileComparison, GitCliProvider, GitError,
    GitProvider, LogPage, LogRequest, PartialStageRequest, PartialStageSelection, Repository,
    RepositoryStatus, Result, StashEntry, StashMutation, StatusStats, Worktree, WorktreeCreate,
    WorktreeRemovalPlan,
};
use crate::workspace::{BufferId, BufferRevision};

const REQUEST_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 128;
const MAX_WORKERS: usize = 4;
/// How often the scheduler re-checks its two channels while Git work is in
/// flight. An idle scheduler blocks instead of polling, so this never runs
/// longer than the Git operations themselves.
const ACTIVE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GitRequestId(u64);

impl GitRequestId {
    #[cfg(test)]
    pub(crate) const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RepositoryGeneration(u64);

impl RepositoryGeneration {
    #[cfg(test)]
    pub(crate) const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RefreshSpec {
    pub staged_paths: Vec<PathBuf>,
    pub branches: bool,
    /// Whether the changed-file list is open, and so whether the line counts
    /// beside its rows are worth another read of both trees.
    pub stats: bool,
    pub staged_diff: bool,
    pub file_diffs: Vec<(PathBuf, DiffScope)>,
    pub worktrees: bool,
    pub log: bool,
    /// Selected log identities that must survive a refresh while reachable.
    pub log_anchors: Vec<String>,
    pub stashes: bool,
}

impl RefreshSpec {
    pub(crate) fn covers(&self, required: &Self) -> bool {
        required
            .staged_paths
            .iter()
            .all(|path| self.staged_paths.contains(path))
            && (!required.branches || self.branches)
            && (!required.stats || self.stats)
            && (!required.staged_diff || self.staged_diff)
            && required
                .file_diffs
                .iter()
                .all(|diff| self.file_diffs.contains(diff))
            && (!required.worktrees || self.worktrees)
            && (!required.log || self.log)
            && required
                .log_anchors
                .iter()
                .all(|oid| self.log_anchors.contains(oid))
            && (!required.stashes || self.stashes)
    }

    pub(crate) fn merge(&mut self, other: &Self) {
        self.staged_paths.extend(other.staged_paths.iter().cloned());
        self.staged_paths.sort();
        self.staged_paths.dedup();
        self.branches |= other.branches;
        self.stats |= other.stats;
        self.staged_diff |= other.staged_diff;
        self.file_diffs.extend(other.file_diffs.iter().cloned());
        self.file_diffs.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| (left.1 == DiffScope::Staged).cmp(&(right.1 == DiffScope::Staged)))
        });
        self.file_diffs.dedup();
        self.worktrees |= other.worktrees;
        self.log |= other.log;
        self.log_anchors.extend(other.log_anchors.iter().cloned());
        self.log_anchors.sort();
        self.log_anchors.dedup();
        self.stashes |= other.stashes;
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BlameSource {
    pub buffer: BufferId,
    pub revision: BufferRevision,
    pub repository: PathBuf,
    pub path: PathBuf,
    pub full_file: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum GitMutation {
    Stage(Vec<PathBuf>),
    Unstage(Vec<PathBuf>),
    Discard(Vec<PathBuf>),
    Checkout {
        branch: String,
    },
    CreateBranch {
        branch: String,
        start: String,
    },
    DeleteBranch {
        plan: Box<BranchDeletionPlan>,
        authorization: DeletionAuthorization,
    },
    Commit {
        message: String,
    },
    Pull,
    /// Replays the current branch's unpushed commits onto its upstream, which
    /// is what a reader confirms after [`GitMutation::Pull`] reports drift in
    /// both directions.
    RebaseOntoUpstream,
    Push {
        branch: String,
    },
    CreateWorktree(WorktreeCreate),
    RemoveWorktree {
        plan: Box<WorktreeRemovalPlan>,
        authorization: DeletionAuthorization,
    },
    Stash(StashMutation),
    PartialStage(Box<PartialStageRequest>),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum MutationIdentity {
    Stage(Vec<PathBuf>),
    Unstage(Vec<PathBuf>),
    Discard(Vec<PathBuf>),
    Checkout(String),
    CreateBranch(String, String),
    DeleteBranch(String),
    Commit(String),
    Pull,
    RebaseOntoUpstream,
    Push(String),
    CreateWorktree(WorktreeCreate),
    RemoveWorktree(PathBuf),
    Stash(StashMutation),
}

impl GitMutation {
    fn duplicate_identity(&self) -> Option<MutationIdentity> {
        Some(match self {
            Self::Stage(paths) => MutationIdentity::Stage(paths.clone()),
            Self::Unstage(paths) => MutationIdentity::Unstage(paths.clone()),
            Self::Discard(paths) => MutationIdentity::Discard(paths.clone()),
            Self::Checkout { branch } => MutationIdentity::Checkout(branch.clone()),
            Self::CreateBranch { branch, start } => {
                MutationIdentity::CreateBranch(branch.clone(), start.clone())
            }
            Self::DeleteBranch { plan, .. } => MutationIdentity::DeleteBranch(plan.branch.clone()),
            Self::Commit { message } => MutationIdentity::Commit(message.clone()),
            Self::Pull => MutationIdentity::Pull,
            Self::RebaseOntoUpstream => MutationIdentity::RebaseOntoUpstream,
            Self::Push { branch } => MutationIdentity::Push(branch.clone()),
            Self::CreateWorktree(request) => MutationIdentity::CreateWorktree(request.clone()),
            Self::RemoveWorktree { plan, .. } => {
                MutationIdentity::RemoveWorktree(plan.path.clone())
            }
            Self::Stash(mutation) => MutationIdentity::Stash(mutation.clone()),
            // Repeating an exact partial request after the first succeeds must
            // run and fail its fingerprint check, not be mistaken for a
            // harmless duplicate while queued behind it.
            Self::PartialStage(_) => return None,
        })
    }
}

impl GitMutation {
    fn label(&self) -> &'static str {
        match self {
            Self::Stage(_) => "stage",
            Self::Unstage(_) => "unstage",
            Self::Discard(_) => "discard",
            Self::Checkout { .. } => "checkout",
            Self::CreateBranch { .. } => "create branch",
            Self::DeleteBranch { .. } => "delete branch",
            Self::Commit { .. } => "commit",
            Self::Pull => "pull",
            Self::RebaseOntoUpstream => "rebase onto upstream",
            Self::Push { .. } => "push",
            Self::CreateWorktree(_) => "create worktree",
            Self::RemoveWorktree { .. } => "remove worktree",
            Self::Stash(StashMutation::Create { .. }) => "create stash",
            Self::Stash(StashMutation::Apply { .. }) => "apply stash",
            Self::Stash(StashMutation::Drop { .. }) => "drop stash",
            Self::PartialStage(request) if request.scope == DiffScope::Staged => "unstage hunk",
            Self::PartialStage(_) => "stage hunk",
        }
    }
}

#[derive(Clone, Debug)]
pub enum GitOperation {
    Discover {
        start: PathBuf,
    },
    Status {
        repository: Repository,
    },
    StagedContent {
        repository: Repository,
        path: PathBuf,
    },
    Diff {
        repository: Repository,
        scope: DiffScope,
        path: Option<PathBuf>,
    },
    FileComparison {
        repository: Repository,
        scope: DiffScope,
        path: PathBuf,
    },
    Branches {
        repository: Repository,
    },
    Worktrees {
        repository: Repository,
    },
    PrepareBranchDeletion {
        repository: Repository,
        branch: String,
        /// The one checkout a cascade is removing on the way to this branch,
        /// which the review tolerates for that reason. `None` is the ordinary
        /// review, which refuses any checkout at all.
        cascade_checkout: Option<PathBuf>,
    },
    PrepareWorktreeRemoval {
        repository: Repository,
        path: PathBuf,
    },
    Log {
        repository: Repository,
        request: LogRequest,
    },
    SearchCommits {
        repository: Repository,
    },
    Stashes {
        repository: Repository,
    },
    PreparePartial {
        repository: Repository,
        selection: Box<PartialStageSelection>,
    },
    CommitDetail {
        repository: Repository,
        oid: String,
    },
    Blame {
        repository: Repository,
        request: BlameRequest,
        source: BlameSource,
    },
    Refresh {
        repository: Repository,
        spec: RefreshSpec,
    },
    Mutate {
        repository: Repository,
        mutation: GitMutation,
        refresh: RefreshSpec,
    },
}

impl GitOperation {
    fn repository_key(&self) -> PathBuf {
        match self {
            Self::Discover { start } => start.clone(),
            Self::Status { repository }
            | Self::StagedContent { repository, .. }
            | Self::Diff { repository, .. }
            | Self::FileComparison { repository, .. }
            | Self::Branches { repository }
            | Self::Worktrees { repository }
            | Self::PrepareBranchDeletion { repository, .. }
            | Self::PrepareWorktreeRemoval { repository, .. }
            | Self::Log { repository, .. }
            | Self::SearchCommits { repository }
            | Self::Stashes { repository }
            | Self::PreparePartial { repository, .. }
            | Self::CommitDetail { repository, .. }
            | Self::Blame { repository, .. }
            | Self::Refresh { repository, .. }
            | Self::Mutate { repository, .. } => repository.common_dir().to_path_buf(),
        }
    }

    pub fn is_mutation(&self) -> bool {
        matches!(self, Self::Mutate { .. })
    }

    /// Whether a failure of this operation leaves a previously successful
    /// answer still on screen.
    ///
    /// [`RepositorySnapshot`] is where the status line, gutter marks, and
    /// branch/worktree/log/stash panels get their data, and a failed refresh
    /// of any of those fields simply does not overwrite it, so the panel
    /// keeps showing what it last successfully read. `CommitDetail`, `Blame`,
    /// `SearchCommits`, and `PreparePartial` answer a one-shot foreground
    /// request instead: nothing is shown until the request succeeds, so a
    /// failure has no prior view to fall back to.
    pub fn refreshes_ambient_snapshot(&self) -> bool {
        !matches!(
            self,
            Self::CommitDetail { .. }
                | Self::Blame { .. }
                | Self::FileComparison { .. }
                | Self::SearchCommits { .. }
                | Self::PreparePartial { .. }
                | Self::PrepareBranchDeletion { .. }
                | Self::PrepareWorktreeRemoval { .. }
        )
    }

    pub fn operation_label(&self) -> &'static str {
        self.label()
    }

    pub fn target(&self) -> PathBuf {
        self.repository_key()
    }

    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Discover { .. } => "discover repository",
            Self::Status { .. } => "refresh status",
            Self::StagedContent { .. } => "read staged base",
            Self::Diff { .. } => "read diff",
            Self::FileComparison { .. } => "read file comparison",
            Self::Branches { .. } => "list branches",
            Self::Worktrees { .. } => "list worktrees",
            Self::PrepareBranchDeletion { .. } => "review branch deletion",
            Self::PrepareWorktreeRemoval { .. } => "review worktree removal",
            Self::Log { .. } => "read log",
            Self::SearchCommits { .. } => "search commits",
            Self::Stashes { .. } => "list stashes",
            Self::PreparePartial { .. } => "prepare partial patch",
            Self::CommitDetail { .. } => "read commit",
            Self::Blame { .. } => "blame live buffer",
            Self::Refresh { .. } => "refresh repository",
            Self::Mutate { mutation, .. } => mutation.label(),
        }
    }

    fn read_key(&self) -> Option<ReadKey> {
        match self {
            Self::Discover { start } => Some(ReadKey::Discover(start.clone())),
            Self::Status { repository } => {
                Some(ReadKey::Status(repository.workdir().to_path_buf()))
            }
            Self::StagedContent { repository, path } => Some(ReadKey::Staged(
                repository.workdir().to_path_buf(),
                path.clone(),
            )),
            Self::Diff {
                repository,
                scope,
                path,
            } => Some(ReadKey::Diff(
                repository.workdir().to_path_buf(),
                *scope,
                path.clone(),
            )),
            Self::FileComparison {
                repository,
                scope,
                path,
            } => Some(ReadKey::FileComparison(
                repository.workdir().to_path_buf(),
                *scope,
                path.clone(),
            )),
            Self::Branches { repository } => {
                Some(ReadKey::Branches(repository.workdir().to_path_buf()))
            }
            Self::Worktrees { repository } => {
                Some(ReadKey::Worktrees(repository.workdir().to_path_buf()))
            }
            Self::PrepareBranchDeletion { .. } | Self::PrepareWorktreeRemoval { .. } => None,
            Self::Log {
                repository,
                request,
            } => Some(ReadKey::Log(
                repository.workdir().to_path_buf(),
                request.clone(),
            )),
            Self::SearchCommits { repository } => {
                Some(ReadKey::SearchCommits(repository.workdir().to_path_buf()))
            }
            Self::Stashes { repository } => {
                Some(ReadKey::Stashes(repository.workdir().to_path_buf()))
            }
            // A live-buffer guard belongs to exactly one request and cannot be
            // shared by read coalescing, even when every text coordinate is
            // otherwise equal.
            Self::PreparePartial { .. } => None,
            Self::CommitDetail { repository, oid } => Some(ReadKey::CommitDetail(
                repository.workdir().to_path_buf(),
                oid.clone(),
            )),
            Self::Blame {
                repository,
                request,
                source,
            } => Some(ReadKey::Blame(
                repository.workdir().to_path_buf(),
                request.clone(),
                source.clone(),
            )),
            Self::Refresh { repository, spec } => Some(ReadKey::Refresh(
                repository.workdir().to_path_buf(),
                refresh_key(spec),
            )),
            Self::Mutate { .. } => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RepositorySnapshot {
    pub repository: Repository,
    pub generation: RepositoryGeneration,
    /// When the service finished reading the fields in this snapshot.
    pub captured_at: Instant,
    /// The subset of ambient state this snapshot authoritatively refreshed.
    pub requested: RefreshSpec,
    pub status: RepositoryStatus,
    /// The line counts for that status, empty where none were asked for.
    pub stats: StatusStats,
    pub head_oid: Option<String>,
    pub staged: Vec<(PathBuf, BaseContent)>,
    pub branches: Option<Vec<Branch>>,
    pub staged_diff: Option<String>,
    pub file_diffs: Vec<(PathBuf, DiffScope, String)>,
    pub worktrees: Option<Vec<Worktree>>,
    pub log: Option<LogPage>,
    pub requested_log_anchors: Vec<String>,
    pub reachable_log_anchors: Vec<String>,
    pub stashes: Option<Vec<StashEntry>>,
}

#[derive(Clone, Debug)]
pub enum GitResponse {
    Discovered(Option<Repository>),
    Status(RepositoryStatus),
    StagedContent {
        path: PathBuf,
        content: BaseContent,
    },
    Diff {
        scope: DiffScope,
        path: Option<PathBuf>,
        text: String,
    },
    FileComparison {
        scope: DiffScope,
        path: PathBuf,
        comparison: FileComparison,
    },
    Branches(Vec<Branch>),
    Worktrees(Vec<Worktree>),
    PreparedBranchDeletion(BranchDeletionPlan),
    PreparedWorktreeRemoval(WorktreeRemovalPlan),
    Log {
        request: LogRequest,
        page: LogPage,
    },
    SearchCommits(CommitSearchResult),
    Stashes(Vec<StashEntry>),
    PreparedPartial(Box<PartialStageRequest>),
    CommitDetail(CommitDetail),
    Blame {
        source: BlameSource,
        lines: Vec<BlameLine>,
    },
    Snapshot(Box<RepositorySnapshot>),
    Mutation {
        mutation: GitMutation,
        applied_paths: Vec<PathBuf>,
        summary: Option<String>,
        failure: Option<GitError>,
        snapshot: Box<Result<RepositorySnapshot>>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitServiceState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    CompletedWithUncertainState,
}

#[derive(Clone, Debug)]
pub struct GitServiceProgress {
    pub id: GitRequestId,
    pub operation: &'static str,
    pub repository: PathBuf,
    pub state: GitServiceState,
    pub started_at: Option<Instant>,
    pub cancellable: bool,
    pub mutation: bool,
}

#[derive(Clone, Debug)]
pub enum GitServiceEvent {
    Progress(GitServiceProgress),
    Completed {
        id: GitRequestId,
        operation: GitOperation,
        result: Box<Result<GitResponse>>,
        state: GitServiceState,
        coalesced: bool,
    },
}

#[derive(Clone)]
pub struct GitServiceHandle {
    requests: SyncSender<Request>,
    next_id: Arc<AtomicU64>,
    cancellations: Arc<Mutex<HashMap<GitRequestId, Arc<AtomicBool>>>>,
    ordered_with_worktrees: bool,
}

impl GitServiceHandle {
    #[cfg(test)]
    pub(crate) fn recording_for_test() -> (Self, Receiver<GitOperation>) {
        let (requests, receiver) = sync_channel::<Request>(REQUEST_CAPACITY);
        let (operations, recorded) = channel();
        std::thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                if operations.send(request.operation).is_err() {
                    break;
                }
            }
        });
        (
            Self {
                requests,
                next_id: Arc::new(AtomicU64::new(1)),
                cancellations: Arc::new(Mutex::new(HashMap::new())),
                ordered_with_worktrees: false,
            },
            recorded,
        )
    }

    pub fn try_submit(&self, operation: GitOperation) -> Result<GitRequestId> {
        let id = GitRequestId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let cancelled = Arc::new(AtomicBool::new(false));
        self.cancellations
            .lock()
            .map_err(|_| GitError::Unavailable {
                detail: "Git service cancellation table is poisoned".to_owned(),
            })?
            .insert(id, Arc::clone(&cancelled));
        let reservation = (self.ordered_with_worktrees
            && !matches!(operation, GitOperation::Discover { .. }))
        .then(|| super::repository_lock::reserve(&operation.repository_key()));
        let request = Request {
            id,
            operation,
            cancelled,
            reservation,
        };
        self.requests.try_send(request).map_err(|error| {
            if let Ok(mut cancellations) = self.cancellations.lock() {
                cancellations.remove(&id);
            }
            match error {
                TrySendError::Full(_) => GitError::Failed {
                    command: "Git service".to_owned(),
                    code: None,
                    stderr: "request queue is full; retry after current Git work advances"
                        .to_owned(),
                },
                TrySendError::Disconnected(_) => GitError::Unavailable {
                    detail: "Git service has stopped".to_owned(),
                },
            }
        })?;
        Ok(id)
    }

    pub fn cancel(&self, id: GitRequestId) -> bool {
        self.cancellations
            .lock()
            .ok()
            .and_then(|entries| entries.get(&id).cloned())
            .is_some_and(|cancelled| {
                cancelled.store(true, Ordering::Release);
                true
            })
    }
}

pub struct GitService;

impl GitService {
    pub fn spawn(provider: GitCliProvider) -> (GitServiceHandle, mpsc::Receiver<GitServiceEvent>) {
        Self::spawn_worker(provider)
    }

    fn spawn_worker<W: GitServiceWorker>(
        worker: W,
    ) -> (GitServiceHandle, mpsc::Receiver<GitServiceEvent>) {
        let (request_tx, request_rx) = sync_channel(REQUEST_CAPACITY);
        let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);
        let cancellations = Arc::new(Mutex::new(HashMap::new()));
        let handle = GitServiceHandle {
            requests: request_tx,
            next_id: Arc::new(AtomicU64::new(1)),
            cancellations: Arc::clone(&cancellations),
            ordered_with_worktrees: worker.uses_repository_process_lock(),
        };
        std::thread::spawn(move || schedule(worker, request_rx, event_tx, cancellations));
        (handle, event_rx)
    }
}

trait GitServiceWorker: Clone + Send + 'static {
    fn execute(
        &self,
        operation: &GitOperation,
        generation: RepositoryGeneration,
        cancellation: Arc<AtomicBool>,
    ) -> Result<GitResponse>;

    fn uses_repository_process_lock(&self) -> bool {
        false
    }
}

impl GitServiceWorker for GitCliProvider {
    fn execute(
        &self,
        operation: &GitOperation,
        generation: RepositoryGeneration,
        cancellation: Arc<AtomicBool>,
    ) -> Result<GitResponse> {
        execute(
            &self.clone().with_cancellation(cancellation),
            operation,
            generation,
        )
    }

    fn uses_repository_process_lock(&self) -> bool {
        true
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum ReadKey {
    Discover(PathBuf),
    Status(PathBuf),
    Staged(PathBuf, PathBuf),
    Diff(PathBuf, DiffScope, Option<PathBuf>),
    FileComparison(PathBuf, DiffScope, PathBuf),
    Branches(PathBuf),
    Worktrees(PathBuf),
    Log(PathBuf, LogRequest),
    SearchCommits(PathBuf),
    Stashes(PathBuf),
    CommitDetail(PathBuf, String),
    Blame(PathBuf, BlameRequest, BlameSource),
    Refresh(PathBuf, String),
}

fn refresh_key(spec: &RefreshSpec) -> String {
    format!(
        "{:?}|{}|{}|{}|{:?}|{}|{}|{:?}|{}",
        spec.staged_paths,
        spec.branches,
        spec.stats,
        spec.staged_diff,
        spec.file_diffs,
        spec.worktrees,
        spec.log,
        spec.log_anchors,
        spec.stashes
    )
}

struct Request {
    id: GitRequestId,
    operation: GitOperation,
    cancelled: Arc<AtomicBool>,
    reservation: Option<super::repository_lock::RepositoryReservation>,
}

type Waiter = (GitRequestId, Arc<AtomicBool>);
type Waiters = Arc<Mutex<Vec<Waiter>>>;

struct Job {
    operation: GitOperation,
    repository: PathBuf,
    waiters: Waiters,
    /// Set once every waiter has cancelled, which abandons the subprocess.
    /// Written and read only under `waiters`, so a request joining an
    /// in-flight read can never attach to an execution that is already being
    /// abandoned. Cloned into the worker, which reads it without the lock.
    execution_cancelled: Arc<AtomicBool>,
    read_key: Option<ReadKey>,
    mutation_key: Option<(PathBuf, MutationIdentity)>,
    reservation: Option<super::repository_lock::RepositoryReservation>,
}

struct Completion {
    repository: PathBuf,
    operation: GitOperation,
    waiters: Waiters,
    read_key: Option<ReadKey>,
    result: Result<GitResponse>,
    mutation: bool,
    mutation_key: Option<(PathBuf, MutationIdentity)>,
    executed: bool,
}

fn schedule<W: GitServiceWorker>(
    worker: W,
    requests: Receiver<Request>,
    events: mpsc::Sender<GitServiceEvent>,
    cancellations: Arc<Mutex<HashMap<GitRequestId, Arc<AtomicBool>>>>,
) {
    let (completion_tx, completion_rx) = channel::<Completion>();
    let mut queues = HashMap::<PathBuf, VecDeque<Job>>::new();
    let mut active_repositories = HashSet::new();
    let mut active_reads = HashMap::<ReadKey, (Waiters, Arc<AtomicBool>)>::new();
    let mut active_mutations = HashSet::<(PathBuf, MutationIdentity)>::new();
    let mut generations = HashMap::<PathBuf, RepositoryGeneration>::new();
    let mut active = 0usize;
    let mut disconnected = false;
    let mut backlog = VecDeque::<Completion>::new();
    loop {
        while let Some(completion) = backlog
            .pop_front()
            .or_else(|| completion_rx.try_recv().ok())
        {
            active = active.saturating_sub(1);
            active_repositories.remove(&completion.repository);
            if let Some(key) = &completion.read_key {
                active_reads.remove(key);
            }
            if let Some(key) = &completion.mutation_key {
                active_mutations.remove(key);
            }
            let waiters = completion
                .waiters
                .lock()
                .map(|waiters| waiters.clone())
                .unwrap_or_default();
            for (position, (id, cancelled)) in waiters.into_iter().enumerate() {
                let cancelled = cancelled.load(Ordering::Acquire);
                let state = if cancelled && completion.mutation && completion.executed {
                    GitServiceState::CompletedWithUncertainState
                } else if cancelled {
                    GitServiceState::Cancelled
                } else if completion.result.is_err()
                    || matches!(
                        &completion.result,
                        Ok(GitResponse::Mutation {
                            failure: Some(_),
                            ..
                        })
                    )
                {
                    GitServiceState::Failed
                } else {
                    GitServiceState::Completed
                };
                let result = if cancelled && (!completion.mutation || !completion.executed) {
                    Err(GitError::Failed {
                        command: completion.operation.label().to_owned(),
                        code: None,
                        stderr: "cancelled; the read result was discarded".to_owned(),
                    })
                } else {
                    completion.result.clone()
                };
                let _ = events.blocking_send(GitServiceEvent::Completed {
                    id,
                    operation: completion.operation.clone(),
                    result: Box::new(result),
                    state,
                    coalesced: position > 0,
                });
                if let Ok(mut entries) = cancellations.lock() {
                    entries.remove(&id);
                }
            }
        }

        while active < MAX_WORKERS {
            let Some(repository) = queues
                .iter()
                .find(|(repository, queue)| {
                    !queue.is_empty() && !active_repositories.contains(*repository)
                })
                .map(|(repository, _)| repository.clone())
            else {
                break;
            };
            let job = queues
                .get_mut(&repository)
                .and_then(VecDeque::pop_front)
                .unwrap();
            let waiters = job
                .waiters
                .lock()
                .map(|waiters| waiters.clone())
                .unwrap_or_default();
            if !waiters.is_empty()
                && waiters
                    .iter()
                    .all(|(_, cancelled)| cancelled.load(Ordering::Acquire))
            {
                for (position, (id, _)) in waiters.into_iter().enumerate() {
                    let _ = events.blocking_send(GitServiceEvent::Completed {
                        id,
                        operation: job.operation.clone(),
                        result: Box::new(Err(GitError::Failed {
                            command: job.operation.label().to_owned(),
                            code: None,
                            stderr: "cancelled before the Git operation started".to_owned(),
                        })),
                        state: GitServiceState::Cancelled,
                        coalesced: position > 0,
                    });
                    if let Ok(mut entries) = cancellations.lock() {
                        entries.remove(&id);
                    }
                }
                continue;
            }
            active_repositories.insert(repository.clone());
            active += 1;
            if let Some(key) = &job.read_key {
                active_reads.insert(
                    key.clone(),
                    (
                        Arc::clone(&job.waiters),
                        Arc::clone(&job.execution_cancelled),
                    ),
                );
            }
            let ids = job
                .waiters
                .lock()
                .map(|waiters| waiters.clone())
                .unwrap_or_default();
            for (id, _) in ids {
                let _ = events.blocking_send(GitServiceEvent::Progress(GitServiceProgress {
                    id,
                    operation: job.operation.label(),
                    repository: repository.clone(),
                    state: GitServiceState::Running,
                    started_at: Some(Instant::now()),
                    cancellable: true,
                    mutation: job.operation.is_mutation(),
                }));
            }
            if let Some(key) = &job.mutation_key {
                active_mutations.insert(key.clone());
            }
            let worker = worker.clone();
            let completion_tx = completion_tx.clone();
            let generation = *generations.entry(repository.clone()).or_default();
            if matches!(job.operation, GitOperation::Mutate { .. }) {
                let next = RepositoryGeneration(generation.0.saturating_add(1));
                generations.insert(repository.clone(), next);
            }
            let generation = generations[&repository];
            std::thread::spawn(move || {
                let mutation = matches!(job.operation, GitOperation::Mutate { .. });
                let execution_cancelled = Arc::clone(&job.execution_cancelled);
                let monitor_cancelled = Arc::clone(&job.execution_cancelled);
                let monitor_waiters = Arc::clone(&job.waiters);
                let monitor_done = Arc::new(AtomicBool::new(false));
                let done = Arc::clone(&monitor_done);
                let monitor = std::thread::spawn(move || {
                    while !done.load(Ordering::Acquire) {
                        // Publishing the decision under the waiters lock makes
                        // it mutually exclusive with a request joining this
                        // read, so a newly attached waiter is either seen here
                        // (and keeps the work alive) or sees the cancellation
                        // and starts its own job instead.
                        let waiters = monitor_waiters
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        let all_cancelled = !waiters.is_empty()
                            && waiters.iter().all(|(_, flag)| flag.load(Ordering::Acquire));
                        if all_cancelled {
                            monitor_cancelled.store(true, Ordering::Release);
                        }
                        drop(waiters);
                        if all_cancelled {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                });
                let (result, executed) = if let Some(reservation) = job.reservation {
                    match reservation.acquire(Some(&execution_cancelled)) {
                        Some(_guard) => (
                            worker.execute(&job.operation, generation, execution_cancelled),
                            true,
                        ),
                        None => (
                            Err(GitError::Cancelled {
                                command: job.operation.label().to_owned(),
                            }),
                            false,
                        ),
                    }
                } else {
                    (
                        worker.execute(&job.operation, generation, execution_cancelled),
                        true,
                    )
                };
                monitor_done.store(true, Ordering::Release);
                let _ = monitor.join();
                let _ = completion_tx.send(Completion {
                    repository: job.repository,
                    operation: job.operation,
                    waiters: job.waiters,
                    read_key: job.read_key,
                    result,
                    mutation,
                    mutation_key: job.mutation_key,
                    executed,
                });
            });
        }

        // Wait for the next thing to do without spinning. Each state has
        // exactly one channel worth blocking on: no worker can report a
        // completion while none is active, and no request can arrive once
        // every handle has been dropped. Only the overlap needs a poll, and
        // that lasts no longer than the Git work itself.
        let next = if disconnected {
            if active == 0 {
                break;
            }
            // The request channel is closed, so `recv_timeout` on it would
            // return `Disconnected` immediately and spin a core until the
            // in-flight work finished. Wait on completions instead.
            backlog.extend(completion_rx.recv_timeout(ACTIVE_POLL_INTERVAL).ok());
            None
        } else if active == 0 {
            // The start pass above leaves every queue empty whenever nothing
            // is active, so a request is the only event that can arrive.
            match requests.recv() {
                Ok(request) => Some(request),
                Err(_) => {
                    disconnected = true;
                    None
                }
            }
        } else {
            match requests.recv_timeout(ACTIVE_POLL_INTERVAL) {
                Ok(request) => Some(request),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    disconnected = true;
                    None
                }
            }
        };
        if let Some(request) = next {
            let repository = request.operation.repository_key();
            let _ = events.blocking_send(GitServiceEvent::Progress(GitServiceProgress {
                id: request.id,
                operation: request.operation.label(),
                repository: repository.clone(),
                state: GitServiceState::Queued,
                // This clock also drives the generic indeterminate status
                // animation while a mutation waits behind earlier work.
                started_at: Some(Instant::now()),
                cancellable: true,
                mutation: request.operation.is_mutation(),
            }));
            let read_key = request.operation.read_key();
            let mutation_key = match &request.operation {
                GitOperation::Mutate {
                    repository,
                    mutation,
                    ..
                } => mutation
                    .duplicate_identity()
                    .map(|identity| (repository.common_dir().to_path_buf(), identity)),
                _ => None,
            };
            if mutation_key.as_ref().is_some_and(|key| {
                active_mutations.contains(key)
                    || queues
                        .values()
                        .flat_map(|queue| queue.iter())
                        .any(|job| job.mutation_key.as_ref() == Some(key))
            }) {
                let error = GitError::Failed {
                    command: request.operation.label().to_owned(),
                    code: None,
                    stderr: "an equivalent mutation is already queued or running".to_owned(),
                };
                let _ = events.blocking_send(GitServiceEvent::Completed {
                    id: request.id,
                    operation: request.operation,
                    result: Box::new(Err(error)),
                    state: GitServiceState::Failed,
                    coalesced: true,
                });
                if let Ok(mut entries) = cancellations.lock() {
                    entries.remove(&request.id);
                }
                continue;
            }
            if let Some(key) = &read_key
                    && let Some((waiters, execution_cancelled)) = active_reads.get(key)
                    && let Ok(mut waiters) = waiters.lock()
                    // Reading the flag under the same lock the monitor takes
                    // to set it keeps this from attaching to an execution that
                    // is already being abandoned, which would report someone
                    // else's cancellation as this request's failure.
                    && !execution_cancelled.load(Ordering::Acquire)
            {
                waiters.push((request.id, request.cancelled));
                continue;
            }
            if let Some(key) = &read_key
                && let Some(waiters) = queues
                    .values_mut()
                    .flat_map(|queue| queue.iter_mut())
                    .find_map(|job| {
                        (job.read_key.as_ref() == Some(key)).then(|| Arc::clone(&job.waiters))
                    })
            {
                if let Ok(mut waiters) = waiters.lock() {
                    waiters.push((request.id, request.cancelled));
                }
                continue;
            }
            queues
                .entry(repository.clone())
                .or_default()
                .push_back(Job {
                    operation: request.operation,
                    repository,
                    waiters: Arc::new(Mutex::new(vec![(request.id, request.cancelled)])),
                    execution_cancelled: Arc::new(AtomicBool::new(false)),
                    read_key,
                    mutation_key,
                    reservation: request.reservation,
                });
        }
    }
}

fn execute(
    provider: &dyn GitProvider,
    operation: &GitOperation,
    generation: RepositoryGeneration,
) -> Result<GitResponse> {
    match operation {
        GitOperation::Discover { start } => provider.discover(start).map(GitResponse::Discovered),
        GitOperation::Status { repository } => provider.status(repository).map(GitResponse::Status),
        GitOperation::StagedContent { repository, path } => provider
            .staged_content(repository, path)
            .map(|content| GitResponse::StagedContent {
                path: path.clone(),
                content,
            }),
        GitOperation::Diff {
            repository,
            scope,
            path,
        } => provider
            .diff(repository, *scope, path.as_deref())
            .map(|text| GitResponse::Diff {
                scope: *scope,
                path: path.clone(),
                text,
            }),
        GitOperation::FileComparison {
            repository,
            scope,
            path,
        } => provider
            .file_comparison(repository, *scope, path)
            .map(|comparison| GitResponse::FileComparison {
                scope: *scope,
                path: path.clone(),
                comparison,
            }),
        GitOperation::Branches { repository } => {
            provider.branches(repository).map(GitResponse::Branches)
        }
        GitOperation::Worktrees { repository } => {
            provider.worktrees(repository).map(GitResponse::Worktrees)
        }
        GitOperation::PrepareBranchDeletion {
            repository,
            branch,
            cascade_checkout,
        } => match cascade_checkout {
            Some(checkout) => {
                provider.prepare_branch_deletion_through(repository, branch, checkout)
            }
            None => provider.prepare_branch_deletion(repository, branch),
        }
        .map(GitResponse::PreparedBranchDeletion),
        GitOperation::PrepareWorktreeRemoval { repository, path } => provider
            .prepare_worktree_removal(repository, path)
            .map(GitResponse::PreparedWorktreeRemoval),
        GitOperation::Log {
            repository,
            request,
        } => provider
            .log_page(repository, request)
            .map(|page| GitResponse::Log {
                request: request.clone(),
                page,
            }),
        GitOperation::SearchCommits { repository } => provider
            .search_commits(repository)
            .map(GitResponse::SearchCommits),
        GitOperation::Stashes { repository } => {
            provider.stashes(repository).map(GitResponse::Stashes)
        }
        GitOperation::PreparePartial {
            repository,
            selection,
        } => provider
            .prepare_partial(repository, selection)
            .map(|request| GitResponse::PreparedPartial(Box::new(request))),
        GitOperation::CommitDetail { repository, oid } => provider
            .commit_detail(repository, oid)
            .map(GitResponse::CommitDetail),
        GitOperation::Blame {
            repository,
            request,
            source,
        } => provider
            .blame(repository, request)
            .map(|lines| GitResponse::Blame {
                source: source.clone(),
                lines,
            }),
        GitOperation::Refresh { repository, spec } => {
            refresh(provider, repository, spec, generation)
                .map(Box::new)
                .map(GitResponse::Snapshot)
        }
        GitOperation::Mutate {
            repository,
            mutation,
            refresh: spec,
        } => {
            let mut applied_paths = Vec::new();
            let outcome: Result<Option<String>> = match mutation {
                GitMutation::Stage(paths) => {
                    mutate_paths(provider, repository, paths, true, &mut applied_paths)
                }
                GitMutation::Unstage(paths) => {
                    mutate_paths(provider, repository, paths, false, &mut applied_paths)
                }
                GitMutation::Discard(paths) => {
                    for path in paths {
                        match provider.discard(repository, path) {
                            Ok(()) => applied_paths.push(path.clone()),
                            Err(error) => {
                                return Ok(mutation_response(
                                    provider,
                                    repository,
                                    mutation,
                                    spec,
                                    generation,
                                    MutationResultParts {
                                        applied_paths,
                                        summary: None,
                                        failure: Some(error),
                                    },
                                ));
                            }
                        }
                    }
                    Ok(None)
                }
                GitMutation::Checkout { branch } => {
                    provider.checkout_branch(repository, branch).map(|()| None)
                }
                GitMutation::CreateBranch { branch, start } => provider
                    .create_branch(repository, branch, start)
                    .map(|()| None),
                GitMutation::DeleteBranch {
                    plan,
                    authorization,
                } => provider
                    .delete_branch_guarded(repository, plan, *authorization)
                    .map(|()| None),
                GitMutation::Commit { message } => provider.commit(repository, message).map(Some),
                GitMutation::Pull => provider.pull(repository).map(Some),
                GitMutation::RebaseOntoUpstream => {
                    provider.rebase_onto_upstream(repository).map(Some)
                }
                GitMutation::Push { branch } => provider.push(repository, branch).map(Some),
                GitMutation::CreateWorktree(request) => {
                    provider.create_worktree(repository, request).map(|()| None)
                }
                GitMutation::RemoveWorktree {
                    plan,
                    authorization,
                } => provider
                    .remove_worktree_guarded(repository, plan, *authorization)
                    .map(|()| None),
                GitMutation::Stash(request) => provider.mutate_stash(repository, request).map(Some),
                GitMutation::PartialStage(request) => {
                    provider.apply_partial(repository, request).map(|()| {
                        applied_paths.push(request.path.clone());
                        None
                    })
                }
            };
            let (summary, failure) = match outcome {
                Ok(summary) => (summary, None),
                Err(error) => (None, Some(error)),
            };
            Ok(mutation_response(
                provider,
                repository,
                mutation,
                spec,
                generation,
                MutationResultParts {
                    applied_paths,
                    summary,
                    failure,
                },
            ))
        }
    }
}

fn mutate_paths(
    provider: &dyn GitProvider,
    repository: &Repository,
    paths: &[PathBuf],
    stage: bool,
    applied: &mut Vec<PathBuf>,
) -> Result<Option<String>> {
    for path in paths {
        let result = if stage {
            provider.stage(repository, path)
        } else {
            provider.unstage(repository, path)
        };
        result?;
        applied.push(path.clone());
    }
    Ok(None)
}

struct MutationResultParts {
    applied_paths: Vec<PathBuf>,
    summary: Option<String>,
    failure: Option<GitError>,
}

fn mutation_response(
    provider: &dyn GitProvider,
    repository: &Repository,
    mutation: &GitMutation,
    spec: &RefreshSpec,
    generation: RepositoryGeneration,
    result: MutationResultParts,
) -> GitResponse {
    GitResponse::Mutation {
        mutation: mutation.clone(),
        applied_paths: result.applied_paths,
        summary: result.summary,
        failure: result.failure,
        snapshot: Box::new(refresh(provider, repository, spec, generation)),
    }
}

fn refresh(
    provider: &dyn GitProvider,
    repository: &Repository,
    spec: &RefreshSpec,
    generation: RepositoryGeneration,
) -> Result<RepositorySnapshot> {
    let status = provider.status(repository)?;
    let stats = if spec.stats {
        // Counts decorate an otherwise complete status. A provider that
        // cannot produce them must not make the repository snapshot itself
        // fail, or opening the changed-file list would also stop branch,
        // gutter, and diff reconciliation.
        provider
            .status_stats(repository, &status)
            .unwrap_or_default()
    } else {
        StatusStats::default()
    };
    let head_oid = if matches!(status.head, super::Head::Unborn(_)) {
        None
    } else {
        provider.head_oid(repository)?
    };
    let staged = spec
        .staged_paths
        .iter()
        .map(|path| {
            provider
                .staged_content(repository, path)
                .map(|content| (path.clone(), content))
        })
        .collect::<Result<Vec<_>>>()?;
    let branches = spec
        .branches
        .then(|| provider.branches(repository))
        .transpose()?;
    let staged_diff = spec
        .staged_diff
        .then(|| provider.diff(repository, DiffScope::Staged, None))
        .transpose()?;
    let file_diffs = spec
        .file_diffs
        .iter()
        .map(|(path, scope)| {
            provider
                .diff(repository, *scope, Some(path))
                .map(|text| (path.clone(), *scope, text))
        })
        .collect::<Result<Vec<_>>>()?;
    let worktrees = spec
        .worktrees
        .then(|| provider.worktrees(repository))
        .transpose()?;
    let log = spec
        .log
        .then(|| provider.log_page(repository, &LogRequest::default()))
        .transpose()?;
    let reachable_log_anchors = if spec.log {
        spec.log_anchors
            .iter()
            .map(|oid| {
                provider
                    .history_contains(repository, oid)
                    .map(|present| (oid, present))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|(_, present)| *present)
            .map(|(oid, _)| oid.clone())
            .collect()
    } else {
        Vec::new()
    };
    let stashes = spec
        .stashes
        .then(|| provider.stashes(repository))
        .transpose()?;
    Ok(RepositorySnapshot {
        repository: repository.clone(),
        generation,
        captured_at: Instant::now(),
        requested: spec.clone(),
        status,
        stats,
        head_oid,
        staged,
        branches,
        staged_diff,
        file_diffs,
        worktrees,
        log,
        requested_log_anchors: spec.log_anchors.clone(),
        reachable_log_anchors,
        stashes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{Divergence, Head, MemoryGitProvider};
    use std::{
        sync::mpsc::{self as std_mpsc, RecvTimeoutError},
        time::Duration,
    };

    #[derive(Clone)]
    struct ControlledWorker {
        started: std_mpsc::Sender<&'static str>,
        release: Arc<AtomicBool>,
        calls: Arc<AtomicU64>,
        mutation_failure: bool,
        ordered_with_worktrees: bool,
        /// Keeps a cancelled job running so tests can observe the window
        /// between the monitor abandoning the work and the scheduler
        /// retiring it.
        ignore_cancellation: bool,
    }

    impl GitServiceWorker for ControlledWorker {
        fn execute(
            &self,
            operation: &GitOperation,
            generation: RepositoryGeneration,
            cancellation: Arc<AtomicBool>,
        ) -> Result<GitResponse> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.started.send(operation.label()).unwrap();
            while !self.release.load(Ordering::Acquire)
                && (self.ignore_cancellation || !cancellation.load(Ordering::Acquire))
            {
                std::thread::sleep(Duration::from_millis(2));
            }
            if !self.ignore_cancellation && cancellation.load(Ordering::Acquire) {
                return Err(GitError::Cancelled {
                    command: operation.label().to_owned(),
                });
            }
            let status = RepositoryStatus {
                head: Head::Branch("main".to_owned()),
                upstream: None,
                divergence: Divergence::default(),
                files: Vec::new(),
            };
            Ok(match operation {
                GitOperation::Status { .. } => GitResponse::Status(status),
                GitOperation::Log { request, .. } => GitResponse::Log {
                    request: request.clone(),
                    page: LogPage {
                        commits: Vec::new(),
                        next: None,
                        total_pages: 1,
                    },
                },
                GitOperation::Refresh { repository, spec } => {
                    GitResponse::Snapshot(Box::new(RepositorySnapshot {
                        repository: repository.clone(),
                        generation,
                        captured_at: Instant::now(),
                        requested: spec.clone(),
                        status,
                        stats: StatusStats::default(),
                        head_oid: Some("head".to_owned()),
                        staged: Vec::new(),
                        branches: None,
                        staged_diff: None,
                        file_diffs: Vec::new(),
                        worktrees: None,
                        log: None,
                        requested_log_anchors: Vec::new(),
                        reachable_log_anchors: Vec::new(),
                        stashes: None,
                    }))
                }
                GitOperation::Mutate {
                    repository,
                    mutation,
                    refresh,
                } => GitResponse::Mutation {
                    mutation: mutation.clone(),
                    applied_paths: Vec::new(),
                    summary: None,
                    failure: self.mutation_failure.then(|| GitError::Failed {
                        command: mutation.label().to_owned(),
                        code: Some(1),
                        stderr: "mutation refused".to_owned(),
                    }),
                    snapshot: Box::new(Ok(RepositorySnapshot {
                        repository: repository.clone(),
                        generation,
                        captured_at: Instant::now(),
                        requested: refresh.clone(),
                        status,
                        stats: StatusStats::default(),
                        head_oid: Some("head".to_owned()),
                        staged: Vec::new(),
                        branches: None,
                        staged_diff: None,
                        file_diffs: Vec::new(),
                        worktrees: None,
                        log: None,
                        requested_log_anchors: Vec::new(),
                        reachable_log_anchors: Vec::new(),
                        stashes: None,
                    })),
                },
                _ => panic!("unexpected operation in controlled worker"),
            })
        }

        fn uses_repository_process_lock(&self) -> bool {
            self.ordered_with_worktrees
        }
    }

    fn worker() -> (
        ControlledWorker,
        std_mpsc::Receiver<&'static str>,
        Arc<AtomicBool>,
        Arc<AtomicU64>,
    ) {
        let (started, receiver) = std_mpsc::channel();
        let release = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicU64::new(0));
        (
            ControlledWorker {
                started,
                release: Arc::clone(&release),
                calls: Arc::clone(&calls),
                mutation_failure: false,
                ordered_with_worktrees: false,
                ignore_cancellation: false,
            },
            receiver,
            release,
            calls,
        )
    }

    fn mutation(repository: &Repository, path: &str) -> GitOperation {
        GitOperation::Mutate {
            repository: repository.clone(),
            mutation: GitMutation::Stage(vec![repository.workdir().join(path)]),
            refresh: RefreshSpec::default(),
        }
    }

    fn completed(receiver: &mut mpsc::Receiver<GitServiceEvent>) -> GitServiceEvent {
        loop {
            let event = receiver.blocking_recv().expect("service stopped early");
            if matches!(event, GitServiceEvent::Completed { .. }) {
                return event;
            }
        }
    }

    #[test]
    fn unavailable_line_counts_do_not_fail_a_repository_snapshot() {
        let repository = Repository::new("/repository");
        let provider = MemoryGitProvider::new(repository.clone()).refusing_stats();

        let snapshot = refresh(
            &provider,
            &repository,
            &RefreshSpec {
                stats: true,
                ..RefreshSpec::default()
            },
            RepositoryGeneration::default(),
        )
        .unwrap();

        assert!(snapshot.stats.is_empty());
        assert_eq!(snapshot.status.head, Head::Branch("main".to_owned()));
    }

    #[test]
    fn request_identities_are_monotonic_and_queue_is_bounded() {
        let (requests, _receiver) = sync_channel(1);
        let handle = GitServiceHandle {
            requests,
            next_id: Arc::new(AtomicU64::new(1)),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
            ordered_with_worktrees: false,
        };
        let first = handle
            .try_submit(GitOperation::Discover {
                start: "/one".into(),
            })
            .unwrap();
        assert!(
            handle
                .try_submit(GitOperation::Discover {
                    start: "/two".into()
                })
                .is_err()
        );
        assert_eq!(first.get(), 1);
    }

    #[test]
    fn a_request_never_coalesces_onto_an_abandoned_read() {
        // The only waiter on an in-flight read cancels, which abandons the
        // subprocess. A request arriving afterwards must get its own run
        // rather than inheriting someone else's cancellation as a failure.
        // The worker ignores cancellation so the read is still registered as
        // active when the second request arrives, which is the racing window.
        let (mut worker, started, release, calls) = worker();
        worker.ignore_cancellation = true;
        let (handle, mut events) = GitService::spawn_worker(worker);
        let repository = Repository::with_common_dir("/one", "/shared/.git");
        let operation = GitOperation::Status {
            repository: repository.clone(),
        };

        let first = handle.try_submit(operation.clone()).unwrap();
        assert_eq!(
            started.recv_timeout(Duration::from_secs(1)).unwrap(),
            "refresh status"
        );
        handle.cancel(first);
        // Let the monitor observe the cancellation while the job still runs.
        std::thread::sleep(Duration::from_millis(50));

        handle.try_submit(operation).unwrap();
        release.store(true, Ordering::Release);
        assert!(matches!(
            completed(&mut events),
            GitServiceEvent::Completed {
                state: GitServiceState::Cancelled,
                ..
            }
        ));
        assert_eq!(
            started.recv_timeout(Duration::from_secs(1)).unwrap(),
            "refresh status"
        );
        let event = completed(&mut events);
        assert!(
            matches!(
                &event,
                GitServiceEvent::Completed {
                    state: GitServiceState::Completed,
                    coalesced: false,
                    ..
                }
            ),
            "{event:?}"
        );
        assert_eq!(calls.load(Ordering::Relaxed), 2, "the read was not re-run");
    }

    #[test]
    fn an_idle_scheduler_blocks_but_still_wakes_for_a_later_request() {
        // With nothing active the scheduler blocks on the request channel
        // instead of polling it, so a request arriving after a long quiet
        // period must still wake it.
        let (worker, started, release, calls) = worker();
        release.store(true, Ordering::Release);
        let (handle, mut events) = GitService::spawn_worker(worker);
        let repository = Repository::with_common_dir("/one", "/shared/.git");

        handle.try_submit(mutation(&repository, "a")).unwrap();
        assert_eq!(
            started.recv_timeout(Duration::from_secs(1)).unwrap(),
            "stage"
        );
        assert!(matches!(
            completed(&mut events),
            GitServiceEvent::Completed {
                state: GitServiceState::Completed,
                ..
            }
        ));

        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(calls.load(Ordering::Relaxed), 1, "idle scheduler ran work");

        handle.try_submit(mutation(&repository, "b")).unwrap();
        assert_eq!(
            started.recv_timeout(Duration::from_secs(1)).unwrap(),
            "stage"
        );
        assert!(matches!(
            completed(&mut events),
            GitServiceEvent::Completed {
                state: GitServiceState::Completed,
                ..
            }
        ));
    }

    #[test]
    fn dropping_the_last_handle_stops_the_scheduler_after_in_flight_work() {
        // A closed request channel reports `Disconnected` immediately, so the
        // scheduler must wait on completions rather than re-polling it while
        // a worker is still running.
        let (worker, started, release, _) = worker();
        let (handle, mut events) = GitService::spawn_worker(worker);
        let repository = Repository::with_common_dir("/one", "/shared/.git");
        handle.try_submit(mutation(&repository, "a")).unwrap();
        assert_eq!(
            started.recv_timeout(Duration::from_secs(1)).unwrap(),
            "stage"
        );

        drop(handle);
        std::thread::sleep(Duration::from_millis(50));
        release.store(true, Ordering::Release);

        assert!(matches!(
            completed(&mut events),
            GitServiceEvent::Completed {
                state: GitServiceState::Completed,
                ..
            }
        ));
        // The scheduler returns once its work drains, dropping the event
        // sender and closing the channel.
        assert!(events.blocking_recv().is_none());
    }

    #[test]
    fn mutations_are_ordered_within_one_common_repository() {
        let (worker, started, release, _) = worker();
        let (handle, mut events) = GitService::spawn_worker(worker);
        let repository = Repository::with_common_dir("/one", "/shared/.git");
        handle.try_submit(mutation(&repository, "a")).unwrap();
        handle.try_submit(mutation(&repository, "b")).unwrap();

        assert_eq!(
            started.recv_timeout(Duration::from_secs(1)).unwrap(),
            "stage"
        );
        assert_eq!(
            started.recv_timeout(Duration::from_millis(80)),
            Err(RecvTimeoutError::Timeout),
            "the second mutation started before the first completed"
        );
        release.store(true, Ordering::Release);
        assert_eq!(
            started.recv_timeout(Duration::from_secs(1)).unwrap(),
            "stage"
        );
        completed(&mut events);
        completed(&mut events);
    }

    #[test]
    fn independent_repositories_use_workers_concurrently() {
        let (worker, started, release, _) = worker();
        let (handle, _events) = GitService::spawn_worker(worker);
        handle
            .try_submit(mutation(&Repository::new("/one"), "a"))
            .unwrap();
        handle
            .try_submit(mutation(&Repository::new("/two"), "b"))
            .unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        started
            .recv_timeout(Duration::from_secs(1))
            .expect("the independent repository did not start concurrently");
        release.store(true, Ordering::Release);
    }

    #[test]
    fn equivalent_refreshes_coalesce_onto_one_worker() {
        let (worker, started, release, calls) = worker();
        let (handle, mut events) = GitService::spawn_worker(worker);
        let repository = Repository::new("/one");
        let operation = GitOperation::Refresh {
            repository,
            spec: RefreshSpec::default(),
        };
        handle.try_submit(operation.clone()).unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        handle.try_submit(operation).unwrap();
        release.store(true, Ordering::Release);
        let first = completed(&mut events);
        let second = completed(&mut events);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(matches!(
            (first, second),
            (
                GitServiceEvent::Completed {
                    coalesced: false,
                    ..
                },
                GitServiceEvent::Completed {
                    coalesced: true,
                    ..
                }
            )
        ));
    }

    #[test]
    fn equivalent_history_pages_coalesce_without_blocking_submission() {
        let (worker, started, release, calls) = worker();
        let (handle, mut events) = GitService::spawn_worker(worker);
        let operation = GitOperation::Log {
            repository: Repository::new("/history"),
            request: LogRequest::default(),
        };
        handle.try_submit(operation.clone()).unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        handle.try_submit(operation).unwrap();
        release.store(true, Ordering::Release);
        let first = completed(&mut events);
        let second = completed(&mut events);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(matches!(
            (first, second),
            (
                GitServiceEvent::Completed {
                    coalesced: false,
                    ..
                },
                GitServiceEvent::Completed {
                    coalesced: true,
                    ..
                }
            )
        ));
    }

    #[test]
    fn cancelling_a_mutation_reports_uncertain_state() {
        let (worker, started, _release, _) = worker();
        let (handle, mut events) = GitService::spawn_worker(worker);
        let id = handle
            .try_submit(mutation(&Repository::new("/one"), "a"))
            .unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(handle.cancel(id));
        assert!(matches!(
            completed(&mut events),
            GitServiceEvent::Completed {
                state: GitServiceState::CompletedWithUncertainState,
                ..
            }
        ));
    }

    #[test]
    fn cancelling_a_queued_mutation_prevents_it_from_starting() {
        let (worker, started, release, calls) = worker();
        let (handle, mut events) = GitService::spawn_worker(worker);
        let repository = Repository::new("/queued-cancellation");
        handle.try_submit(mutation(&repository, "first")).unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        let cancelled = handle.try_submit(mutation(&repository, "second")).unwrap();
        assert!(handle.cancel(cancelled));
        release.store(true, Ordering::Release);

        let first = completed(&mut events);
        let second = completed(&mut events);
        assert!(matches!(
            first,
            GitServiceEvent::Completed {
                state: GitServiceState::Completed,
                ..
            }
        ));
        assert!(matches!(
            second,
            GitServiceEvent::Completed {
                id,
                state: GitServiceState::Cancelled,
                ..
            } if id == cancelled
        ));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn cancellation_interrupts_a_repository_reservation_wait() {
        let repository = Repository::with_common_dir("/waiting", "/held/.git");
        let holder_repository = repository.common_dir().to_path_buf();
        let (held_tx, held_rx) = std_mpsc::channel();
        let (release_holder_tx, release_holder_rx) = std_mpsc::channel();
        let holder = std::thread::spawn(move || {
            let _guard = super::super::repository_lock::acquire(&holder_repository);
            held_tx.send(()).unwrap();
            release_holder_rx.recv().unwrap();
        });
        held_rx.recv().unwrap();
        let (mut worker, started, _release, calls) = worker();
        worker.ordered_with_worktrees = true;
        let (handle, mut events) = GitService::spawn_worker(worker);
        let id = handle.try_submit(mutation(&repository, "path")).unwrap();
        loop {
            if matches!(
                events.blocking_recv().unwrap(),
                GitServiceEvent::Progress(GitServiceProgress {
                    id: progress_id,
                    state: GitServiceState::Running,
                    ..
                }) if progress_id == id
            ) {
                break;
            }
        }
        assert!(handle.cancel(id));

        assert!(matches!(
            completed(&mut events),
            GitServiceEvent::Completed {
                id: completed_id,
                state: GitServiceState::Cancelled,
                ..
            } if completed_id == id
        ));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            started.recv_timeout(Duration::from_millis(30)),
            Err(RecvTimeoutError::Timeout)
        );
        release_holder_tx.send(()).unwrap();
        holder.join().unwrap();
    }

    #[test]
    fn mutation_failure_response_has_failed_terminal_state() {
        let (mut worker, started, release, _) = worker();
        worker.mutation_failure = true;
        release.store(true, Ordering::Release);
        let (handle, mut events) = GitService::spawn_worker(worker);
        handle
            .try_submit(mutation(&Repository::new("/failed-mutation"), "path"))
            .unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            completed(&mut events),
            GitServiceEvent::Completed {
                state: GitServiceState::Failed,
                ..
            }
        ));
    }

    #[test]
    fn cancelling_a_read_discards_its_result_without_uncertainty() {
        let (worker, started, _release, _) = worker();
        let (handle, mut events) = GitService::spawn_worker(worker);
        let id = handle
            .try_submit(GitOperation::Status {
                repository: Repository::new("/one"),
            })
            .unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(handle.cancel(id));
        let GitServiceEvent::Completed { result, state, .. } = completed(&mut events) else {
            unreachable!()
        };
        assert!(matches!(*result, Err(GitError::Failed { .. })));
        assert_eq!(state, GitServiceState::Cancelled);
    }

    #[test]
    fn an_equivalent_mutation_is_refused_instead_of_started_twice() {
        let (worker, started, release, calls) = worker();
        let (handle, mut events) = GitService::spawn_worker(worker);
        let operation = mutation(&Repository::new("/one"), "a");
        handle.try_submit(operation.clone()).unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        handle.try_submit(operation).unwrap();

        let GitServiceEvent::Completed {
            result, coalesced, ..
        } = completed(&mut events)
        else {
            unreachable!()
        };
        assert!(matches!(*result, Err(GitError::Failed { .. })));
        assert!(coalesced);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        release.store(true, Ordering::Release);
    }
}
