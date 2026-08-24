// SPDX-License-Identifier: MPL-2.0

//! Editor coordination for Git status, history, branches, worktrees, and mutations.

// Application-module dependencies:
use super::{
    App, Axis, BindingTarget, BlameLine, BlameRequest, BlameSource, Branch,
    BranchDeletionConfirmation, Buffer, BufferKind, BufferRevisionGuard, COMMIT_INSTRUCTIONS,
    ColonCommand, CommitDetail, CommitSearchResult, CommitSummary, DiffScope, DiffSession,
    DiffSide, Duration, FileComparison, GeneralWorktreeRow, GeneratedViewIdentity,
    GitDiscardConfirmation, GitMutation, GitOperation, GitProvider, GitRequestId, GitResponse,
    GitServiceEvent, GitServiceHandle, GitServiceProgress, GitServiceState, GitStashConfirmation,
    GitTracker, HashMap, HashSet, Instant, KeyCode, KeyStroke, LineChange, ListAction, ListPicker,
    LogCursor, LogPage, LogRequest, LogViewRequest, MAX_BLAME_INPUT_BYTES, MAX_BLAME_LINES,
    MAX_DIFF_BYTES, Mode, PartialStageSelection, PatchHunk, Path, PathBuf, PickerItem, PromptKind,
    PullRebaseConfirmation, RefreshSpec, Repository, RepositoryGeneration, RepositorySnapshot,
    Result, Selection, StashEntry, StashMutation, StashScope, StatusSide, WorkspaceSwitchRequest,
    Worktree, WorktreeCreate, WorktreeRemovalConfirmation, buffer_language, commit_message_body,
    is_refreshed_projection, selection_is_deliberate,
};

/// Git-service bookkeeping and semantic row identities owned by the editor's
/// Git integration.
///
/// `App` still coordinates buffers, panes, and prompts, while this component
/// owns the state that exists only to reconcile asynchronous Git work and to
/// keep generated Git views tied to typed repository objects.
pub(super) struct GitWorkflowState {
    /// Whether repository discovery has answered for the current project.
    /// Kept separate from `GitTracker::repository`: `None` means "not a
    /// repository" only after discovery finishes.
    discovery_complete: bool,
    generation: RepositoryGeneration,
    head_oid: Option<String>,
    progress: HashMap<GitRequestId, GitServiceProgress>,
    action_origins: HashMap<GitRequestId, u64>,
    snapshot_stale: bool,
    /// Async diff projections keyed by their durable buffer arena identity.
    diff_buffers: HashMap<usize, (PathBuf, DiffScope)>,
    index_buffer: Option<usize>,
    index_open_requests: HashSet<GitRequestId>,
    /// Which file each row of the changed-file list stands for, indexed by
    /// document row. Replaced whenever the list's text is.
    status_entries: Vec<Option<crate::git::StatusEntry>>,
    /// Where each row of that list keeps its two line counts, indexed the same
    /// way. Replaced with the text, because a column measured against rows
    /// that have been rewritten would paint the wrong characters.
    status_counts: Vec<Option<crate::git::CountColumns>>,
    /// The projected branch list, indexed by document row: the branch each row
    /// acts on and the columns its Git annotations occupy.
    branch_rows: Vec<crate::git::BranchRow>,
    worktree_rows: Vec<GeneralWorktreeRow>,
    /// Commits on the page currently displayed, not the whole loaded history.
    log_rows: Vec<CommitSummary>,
    log_next: Option<LogCursor>,
    /// The cursor that produced each visited page, indexed by page number.
    /// Page zero is the tip of the branch and has no cursor.
    log_cursors: Vec<Option<LogCursor>>,
    log_page: usize,
    log_requests: HashMap<GitRequestId, LogViewRequest>,
    blame_rows: Vec<BlameLine>,
    stash_rows: Vec<StashEntry>,
    patch_hunks: HashMap<usize, Vec<PatchHunk>>,
    /// Actionable refusal retained for a visible diff whose patch shape is
    /// outside Runyte's conservative partial-stage surface.
    patch_errors: HashMap<usize, String>,
    partial_guards: HashMap<usize, Vec<BufferRevisionGuard>>,
}

impl Default for GitWorkflowState {
    fn default() -> Self {
        Self {
            discovery_complete: false,
            generation: RepositoryGeneration::default(),
            head_oid: None,
            progress: HashMap::new(),
            action_origins: HashMap::new(),
            snapshot_stale: false,
            diff_buffers: HashMap::new(),
            index_buffer: None,
            index_open_requests: HashSet::new(),
            status_entries: Vec::new(),
            status_counts: Vec::new(),
            branch_rows: Vec::new(),
            worktree_rows: Vec::new(),
            log_rows: Vec::new(),
            log_next: None,
            log_cursors: vec![None],
            log_page: 0,
            log_requests: HashMap::new(),
            blame_rows: Vec::new(),
            stash_rows: Vec::new(),
            patch_hunks: HashMap::new(),
            patch_errors: HashMap::new(),
            partial_guards: HashMap::new(),
        }
    }
}

impl GitWorkflowState {
    pub(super) fn discovery_complete(&self) -> bool {
        self.discovery_complete
    }

    pub(super) fn status_counts(&self) -> &[Option<crate::git::CountColumns>] {
        &self.status_counts
    }

    pub(super) fn branch_rows(&self) -> &[crate::git::BranchRow] {
        &self.branch_rows
    }

    #[cfg(test)]
    pub(super) fn set_discovery_complete(&mut self, complete: bool) {
        self.discovery_complete = complete;
    }

    #[cfg(test)]
    pub(super) fn progress(&self) -> &HashMap<GitRequestId, GitServiceProgress> {
        &self.progress
    }

    #[cfg(test)]
    pub(super) fn progress_mut(&mut self) -> &mut HashMap<GitRequestId, GitServiceProgress> {
        &mut self.progress
    }

    #[cfg(test)]
    pub(super) fn action_origins_mut(&mut self) -> &mut HashMap<GitRequestId, u64> {
        &mut self.action_origins
    }

    #[cfg(test)]
    pub(super) fn snapshot_stale(&self) -> bool {
        self.snapshot_stale
    }

    #[cfg(test)]
    pub(super) fn index_buffer(&self) -> Option<usize> {
        self.index_buffer
    }

    #[cfg(test)]
    pub(super) fn log_rows(&self) -> &[CommitSummary] {
        &self.log_rows
    }

    #[cfg(test)]
    pub(super) fn log_cursors(&self) -> &[Option<LogCursor>] {
        &self.log_cursors
    }

    #[cfg(test)]
    pub(super) fn log_page(&self) -> usize {
        self.log_page
    }

    #[cfg(test)]
    pub(super) fn log_requests_mut(&mut self) -> &mut HashMap<GitRequestId, LogViewRequest> {
        &mut self.log_requests
    }

    #[cfg(test)]
    pub(super) fn stash_rows(&self) -> &[StashEntry] {
        &self.stash_rows
    }

    #[cfg(test)]
    pub(super) fn partial_guards(&self) -> &HashMap<usize, Vec<BufferRevisionGuard>> {
        &self.partial_guards
    }

    #[cfg(test)]
    pub(super) fn partial_guards_mut(&mut self) -> &mut HashMap<usize, Vec<BufferRevisionGuard>> {
        &mut self.partial_guards
    }
}

impl App {
    /// Asks Git which repository the project sits in, and reads what it says
    /// about the files already open.
    ///
    /// Everything here is best-effort. A project outside Git, a Git that
    /// cannot be found, and a repository that refuses to answer all leave the
    /// editor working exactly as it did before, with no gutter and no branch
    /// in the status line.
    pub(super) fn attach_repository(&mut self) {
        let project_root = self.project_root.clone();
        let Some((tracker, provider)) = self.git_ports() else {
            self.git_state.discovery_complete = true;
            return;
        };
        let repository = provider.discover(&project_root).unwrap_or_default();
        tracker.attach(repository);
        let _ = tracker.refresh_status(provider);
        self.git_state.discovery_complete = true;
        let paths = self
            .buffers
            .iter()
            .filter_map(|buffer| buffer.path.clone())
            .collect::<Vec<_>>();
        for path in paths {
            self.track_in_git(&path);
        }
    }

    /// Installs the production asynchronous Git service after the first
    /// standalone frame, then discovers the repository without blocking the
    /// editor thread.
    pub fn attach_git_service(&mut self, service: GitServiceHandle) {
        self.ports.git_service = Some(service);
        self.ports.git = None;
        self.git_state.discovery_complete = false;
        let _ = self.request_git(GitOperation::Discover {
            start: self.project_root.clone(),
        });
    }

    pub(super) fn has_git(&self) -> bool {
        self.ports.git_service.is_some() || self.ports.git.is_some()
    }

    fn request_git(&mut self, operation: GitOperation) -> Option<GitRequestId> {
        self.request_git_for_action(operation, self.active_action_id)
    }

    fn request_git_for_action(
        &mut self,
        operation: GitOperation,
        action: Option<u64>,
    ) -> Option<GitRequestId> {
        let service = self.ports.git_service.as_ref()?;
        match service.try_submit(operation) {
            Ok(id) => {
                if let Some(action) = action {
                    self.git_state.action_origins.insert(id, action);
                }
                Some(id)
            }
            Err(error) => {
                self.error(error.to_string());
                None
            }
        }
    }

    fn workspace_contains_path(&self, path: &Path) -> bool {
        crate::path_safety::ensure_within_root(&self.project_root, path).is_ok()
    }

    pub(super) fn git_refresh_spec(&self, repository: &Repository) -> RefreshSpec {
        let visible = self
            .panes
            .values()
            .map(|pane| pane.buffer)
            .collect::<HashSet<_>>();
        let mut staged_paths = self
            .buffers
            .iter()
            .enumerate()
            .filter_map(|(index, buffer)| {
                (!self.closed_buffers.contains(&index) && buffer.kind == BufferKind::File)
                    .then_some(buffer.path.as_ref())
                    .flatten()
                    .filter(|path| self.workspace_contains_path(path) && repository.contains(path))
                    .cloned()
            })
            .collect::<Vec<_>>();
        staged_paths.sort();
        staged_paths.dedup();
        let mut file_diffs = self
            .git_state
            .diff_buffers
            .iter()
            .filter(|(index, _)| visible.contains(index) && !self.closed_buffers.contains(index))
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        file_diffs.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| (left.1 == DiffScope::Staged).cmp(&(right.1 == DiffScope::Staged)))
        });
        file_diffs.dedup();
        let mut log_anchors = self
            .panes
            .values()
            .filter(|pane| self.buffers[pane.buffer].is_git_log())
            .filter_map(|pane| {
                let line = self.buffers[pane.buffer].offset_to_row(pane.head());
                Self::git_log_line_to_row(line)
                    .and_then(|row| self.git_state.log_rows.get(row))
                    .map(|commit| commit.oid.clone())
            })
            .collect::<Vec<_>>();
        log_anchors.sort();
        log_anchors.dedup();
        RefreshSpec {
            staged_paths,
            branches: visible.iter().any(|index| {
                !self.closed_buffers.contains(index) && self.buffers[*index].is_git_branches()
            }),
            // On the buffer rather than on a pane showing it, because the list
            // is rewritten from the same condition: one that is open but
            // covered still has its rows replaced, and rows without their
            // numbers would lose the column until the next refresh.
            stats: self.git_status_buffer().is_some(),
            staged_diff: self.git_state.index_buffer.is_some_and(|index| {
                visible.contains(&index) && !self.closed_buffers.contains(&index)
            }),
            file_diffs,
            worktrees: self
                .panes
                .values()
                .any(|pane| self.buffers[pane.buffer].is_git_worktrees()),
            // Only the first page tracks the branch tip. Every later page sits
            // behind a commit cursor, so its history cannot change under the
            // person and re-reading it would just fight their paging.
            log: self.git_state.log_page == 0
                && self
                    .panes
                    .values()
                    .any(|pane| self.buffers[pane.buffer].is_git_log()),
            log_anchors,
            stashes: self
                .panes
                .values()
                .any(|pane| self.buffers[pane.buffer].is_git_stash()),
        }
    }

    fn request_git_refresh(&mut self) -> bool {
        let Some(repository) = self.git.repository().cloned() else {
            return false;
        };
        let spec = self.git_refresh_spec(&repository);
        self.request_git(GitOperation::Refresh { repository, spec })
            .is_some()
    }

    pub(crate) fn periodic_git_refresh_seconds(&self) -> usize {
        self.config.git.refresh_interval_seconds
    }

    pub(crate) fn has_visible_git_state(&self) -> bool {
        self.panes.values().any(|pane| {
            let buffer = &self.buffers[pane.buffer];
            is_refreshed_projection(buffer)
                || buffer.path.as_deref().is_some_and(|path| {
                    self.workspace_contains_path(path)
                        && self
                            .git
                            .repository()
                            .is_some_and(|repository| repository.contains(path))
                })
        })
    }

    /// Whether replacing a Git projection right now would destroy work the
    /// person can currently see.
    ///
    /// A refresh rewrites the projection's text and rebuilds its selection
    /// from a single row identity, so an unfinished command line and every
    /// search match would vanish under the cursor. Deferring costs nothing:
    /// the timer does not record the skipped tick, so the refresh runs as
    /// soon as the interaction ends, and `:git-refresh` stays available.
    pub(super) fn interaction_defers_git_refresh(&self) -> bool {
        // Every prompt, including `/`, `s`, and `S`, opens in command mode.
        if self.mode == Mode::Command || self.has_input_overlay() {
            return true;
        }
        // A refresh rebuilds the projection and moves the cursor to the
        // nearest surviving row, which is disruptive to walk into while
        // reading or navigating. Waiting out the refresh interval since the
        // last keystroke means it only lands once the person has paused.
        if Instant::now().saturating_duration_since(self.last_interaction)
            < Duration::from_secs(self.periodic_git_refresh_seconds() as u64)
        {
            return true;
        }
        // Only selections inside a projection are at risk. A selection in a
        // source file survives a refresh, which touches its gutter marks
        // rather than its text.
        self.panes.values().any(|pane| {
            let buffer = &self.buffers[pane.buffer];
            is_refreshed_projection(buffer) && selection_is_deliberate(buffer, &pane.selection)
        })
    }

    pub(crate) fn request_periodic_git_refresh(&mut self) -> bool {
        if self
            .git_state
            .progress
            .values()
            .any(|progress| progress.mutation)
            || self.interaction_defers_git_refresh()
        {
            return false;
        }
        self.request_git_refresh()
    }

    pub fn apply_git_service_event(&mut self, event: GitServiceEvent) {
        match event {
            GitServiceEvent::Progress(progress) => {
                if progress.state == GitServiceState::Completed {
                    self.git_state.progress.remove(&progress.id);
                } else {
                    self.git_state.progress.insert(progress.id, progress);
                }
            }
            GitServiceEvent::Completed {
                id,
                operation,
                result,
                state,
                ..
            } => {
                self.git_state.progress.remove(&id);
                let action = self.git_state.action_origins.remove(&id);
                let explicit_index_open = self.git_state.index_open_requests.remove(&id);
                let log_view_request = self.git_state.log_requests.remove(&id);
                if matches!(
                    state,
                    GitServiceState::Cancelled | GitServiceState::CompletedWithUncertainState
                ) {
                    self.git_state.snapshot_stale = true;
                    let _ = self.request_git_refresh();
                }
                match *result {
                    Ok(response) => {
                        self.apply_git_response(
                            operation,
                            response,
                            state,
                            explicit_index_open,
                            log_view_request,
                            action,
                        );
                    }
                    Err(error) => {
                        if matches!(&operation, GitOperation::Discover { .. }) {
                            self.git_state.discovery_complete = true;
                        }
                        if let GitOperation::PreparePartial { selection, .. } = &operation
                            && let (Some((buffer, _)), Some(guard)) =
                                (selection.buffer, selection.guard.as_ref())
                            && let Some(index) = buffer.index()
                        {
                            self.forget_partial_guard(index, guard.id(), true);
                        }
                        let message = if operation.refreshes_ambient_snapshot() {
                            self.git_state.snapshot_stale = true;
                            format!("{error}; showing the last known Git state")
                        } else {
                            error.to_string()
                        };
                        if matches!(operation, GitOperation::Mutate { .. }) {
                            self.mark_action_feedback_failed(action, &message);
                        }
                        self.error_from("Git", "Git operation failed", message);
                    }
                }
            }
        }
    }

    pub(super) fn apply_git_response(
        &mut self,
        operation: GitOperation,
        response: GitResponse,
        state: GitServiceState,
        explicit_index_open: bool,
        log_view_request: Option<LogViewRequest>,
        action: Option<u64>,
    ) {
        match response {
            GitResponse::Discovered(repository) => {
                self.git_state.discovery_complete = true;
                self.git.attach(repository.clone());
                if let Some(repository) = repository {
                    let spec = self.git_refresh_spec(&repository);
                    let _ = self.request_git(GitOperation::Refresh { repository, spec });
                }
            }
            GitResponse::Status(status) => {
                self.git.apply_status(status);
                self.git_state.snapshot_stale = false;
                self.refresh_git_status_buffer();
            }
            GitResponse::StagedContent { path, content } => {
                self.git.apply_staged_content(path, content);
                self.git_state.snapshot_stale = false;
            }
            GitResponse::Diff { scope, path, text } => {
                self.open_git_diff_result(scope, path, text);
            }
            GitResponse::FileComparison {
                scope,
                path,
                comparison,
            } => self.open_git_file_comparison_result(scope, path, comparison),
            GitResponse::Branches(branches) => self.open_git_branches_result(branches),
            GitResponse::Worktrees(worktrees) => {
                let activate = !self.active_buffer().is_git_worktrees();
                self.open_git_worktrees_result(worktrees, activate);
            }
            GitResponse::Log { request, page } => {
                if request.cursor.is_some()
                    && log_view_request
                        .and_then(|request| request.buffer)
                        .is_none_or(|buffer| {
                            self.closed_buffers.contains(&buffer)
                                || !self.panes.values().any(|pane| pane.buffer == buffer)
                        })
                {
                    return;
                }
                let activate = !self.active_buffer().is_git_log() && request.cursor.is_none();
                let target = log_view_request.map_or(0, |request| request.page);
                self.open_git_log_result(request, page, target, activate);
            }
            GitResponse::SearchCommits(result) => self.open_git_commit_search_result(result),
            GitResponse::Stashes(entries) => self.open_git_stashes_result(entries, true),
            GitResponse::PreparedPartial(request) => {
                if request
                    .guard
                    .as_ref()
                    .is_some_and(|guard| !guard.is_valid())
                    || self.has_unsaved_changes(&request.path)
                    || request.buffer.is_some_and(|(buffer, revision)| {
                        buffer.index().is_none_or(|index| {
                            self.closed_buffers.contains(&index)
                                || self.buffers[index].revision() != revision.get()
                                || self.buffers[index].dirty
                                || self.buffers[index].path.as_deref()
                                    != Some(request.path.as_path())
                        })
                    })
                {
                    if let Some(index) = request.buffer.and_then(|(buffer, _)| buffer.index()) {
                        self.invalidate_partial_guards(index);
                    }
                    self.error("stale partial-stage request; the originating buffer changed");
                    return;
                }
                let Some(repository) = self.git.repository().cloned() else {
                    return;
                };
                let refresh = self.git_refresh_spec(&repository);
                let tracked_guard = request
                    .buffer
                    .zip(request.guard.as_ref().map(BufferRevisionGuard::id))
                    .and_then(|((buffer, _), guard)| buffer.index().map(|index| (index, guard)));
                if self
                    .request_git_for_action(
                        GitOperation::Mutate {
                            repository,
                            mutation: GitMutation::PartialStage(request),
                            refresh,
                        },
                        action,
                    )
                    .is_none()
                    && let Some((buffer, guard)) = tracked_guard
                {
                    self.forget_partial_guard(buffer, guard, true);
                }
            }
            GitResponse::CommitDetail(detail) => self.open_git_commit_detail_result(detail),
            GitResponse::Blame { source, lines } => self.open_git_blame_result(source, lines),
            GitResponse::Snapshot(snapshot) => {
                self.apply_repository_snapshot(*snapshot, true, explicit_index_open);
            }
            GitResponse::Mutation {
                mutation,
                applied_paths,
                summary,
                failure,
                snapshot,
            } => {
                if let Ok(snapshot) = *snapshot {
                    self.apply_repository_snapshot(snapshot, false, false);
                } else {
                    self.git_state.snapshot_stale = true;
                }
                self.apply_git_mutation_result(
                    mutation,
                    applied_paths,
                    summary,
                    failure,
                    state,
                    action,
                );
            }
        }
        let _ = operation;
    }

    pub(super) fn apply_repository_snapshot(
        &mut self,
        snapshot: RepositorySnapshot,
        reload_external_head: bool,
        open_index: bool,
    ) {
        if snapshot.generation < self.git_state.generation {
            return;
        }
        let head_changed =
            self.git_state.head_oid.is_some() && self.git_state.head_oid != snapshot.head_oid;
        self.git_state.generation = snapshot.generation;
        self.git_state.head_oid = snapshot.head_oid;
        self.git.apply_snapshot(
            snapshot.repository,
            snapshot.status,
            snapshot.stats,
            snapshot.staged,
        );
        self.git_state.snapshot_stale = false;
        if head_changed && reload_external_head {
            self.reload_clean_repository_buffers();
        }
        self.refresh_git_status_buffer();
        if let Some(branches) = snapshot.branches {
            self.refresh_git_branches_from(branches, "");
        }
        if let Some(diff) = snapshot.staged_diff {
            self.update_git_index_result(diff, open_index);
        }
        for (path, scope, diff) in snapshot.file_diffs {
            self.refresh_git_diff_result(scope, path, diff);
        }
        if let Some(worktrees) = snapshot.worktrees {
            self.open_git_worktrees_result(worktrees, false);
        }
        let current_log_anchors = self
            .panes
            .values()
            .filter(|pane| self.buffers[pane.buffer].is_git_log())
            .filter_map(|pane| {
                let line = self.buffers[pane.buffer].offset_to_row(pane.head());
                Self::git_log_line_to_row(line)
                    .and_then(|row| self.git_state.log_rows.get(row))
                    .map(|commit| commit.oid.clone())
            })
            .collect::<Vec<_>>();
        let log_projection_is_current = current_log_anchors
            .iter()
            .all(|oid| snapshot.requested_log_anchors.contains(oid));
        if let Some(log) = snapshot.log
            && log_projection_is_current
            && self.panes.values().any(|pane| {
                !self.closed_buffers.contains(&pane.buffer)
                    && self.buffers[pane.buffer].is_git_log()
            })
        {
            self.open_git_log_result(LogRequest::default(), log, 0, false);
        }
        if let Some(stashes) = snapshot.stashes
            && self.panes.values().any(|pane| {
                !self.closed_buffers.contains(&pane.buffer)
                    && self.buffers[pane.buffer].is_git_stash()
            })
        {
            self.open_git_stashes_result(stashes, false);
        }
    }

    pub(super) fn apply_git_mutation_result(
        &mut self,
        mutation: GitMutation,
        applied_paths: Vec<PathBuf>,
        summary: Option<String>,
        failure: Option<crate::git::GitError>,
        state: GitServiceState,
        action: Option<u64>,
    ) {
        let created_worktree = match &mutation {
            GitMutation::CreateWorktree(request) => Some(request.destination.clone()),
            _ => None,
        };
        if let GitMutation::PartialStage(request) = &mutation
            && let (Some((buffer, _)), Some(guard)) = (request.buffer, request.guard.as_ref())
            && let Some(index) = buffer.index()
        {
            self.forget_partial_guard(index, guard.id(), false);
        }
        if matches!(
            mutation,
            GitMutation::Checkout { .. }
                | GitMutation::CreateBranch { .. }
                | GitMutation::Pull
                | GitMutation::RebaseOntoUpstream
        ) {
            self.reload_clean_repository_buffers();
        }
        if matches!(
            mutation,
            GitMutation::Stash(StashMutation::Create { .. } | StashMutation::Apply { .. })
        ) {
            self.reload_clean_repository_buffers();
        }
        if matches!(mutation, GitMutation::Discard(_)) {
            self.reload_git_paths(&applied_paths);
        }
        if matches!(mutation, GitMutation::Commit { .. })
            && failure.is_none()
            && let Some(buffer) = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
                (!self.closed_buffers.contains(&index) && buffer.is_commit_message())
                    .then_some(index)
            })
        {
            let _ = self.buffers[buffer].discard_changes_to("");
            self.close_buffer(buffer);
            self.return_from_commit();
        }
        let uncertain = state == GitServiceState::CompletedWithUncertainState;
        // A pull that found commits on both sides is an offer rather than a
        // failure, and it is only one here because the provider has no way to
        // ask. The snapshot alongside it has already landed, so the branch list
        // shows the drift the offer is about.
        if let Some(error) = &failure
            && matches!(mutation, GitMutation::Pull)
            && !uncertain
            && matches!(error, crate::git::GitError::Diverged { .. })
        {
            self.report_pull_failure(error);
            return;
        }
        if let Some(error) = failure {
            let prefix = if applied_paths.is_empty() {
                String::new()
            } else {
                format!("{} path(s) changed before failure; ", applied_paths.len())
            };
            let message = format!(
                "{prefix}{error}{}",
                if uncertain {
                    "; outcome may be partial; repository reconciliation was scheduled"
                } else {
                    ""
                }
            );
            self.mark_action_feedback_failed(action, &message);
            self.error_from("Git", "Git mutation failed", message);
            return;
        }
        let producer_summary = summary
            .map(|summary| summary.trim().to_owned())
            .filter(|summary| !summary.is_empty());
        let message = producer_summary
            .as_deref()
            .and_then(|summary| summary.lines().next().map(str::trim))
            .filter(|summary| !summary.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| match mutation {
                GitMutation::Stage(_) => format!("staged {} file(s)", applied_paths.len()),
                GitMutation::Unstage(_) => format!("unstaged {} file(s)", applied_paths.len()),
                GitMutation::Discard(_) => {
                    format!("discarded changes to {} file(s)", applied_paths.len())
                }
                GitMutation::Checkout { branch } => format!("checked out {branch}"),
                GitMutation::CreateBranch { branch, start } => {
                    format!("created {branch} from {start}")
                }
                GitMutation::DeleteBranch { branch, .. } => format!("deleted branch {branch}"),
                GitMutation::Commit { .. } => "committed".to_owned(),
                GitMutation::Pull => "pull completed".to_owned(),
                GitMutation::RebaseOntoUpstream => "replayed onto the upstream".to_owned(),
                GitMutation::Push { branch } => format!("pushed {branch}"),
                GitMutation::CreateWorktree(request) => {
                    format!("created worktree at {}", request.destination.display())
                }
                GitMutation::RemoveWorktree(path) => {
                    format!(
                        "removed worktree {}; no branch was deleted",
                        crate::git::display_path(&path)
                    )
                }
                GitMutation::Stash(StashMutation::Create { .. }) => "stash created".to_owned(),
                GitMutation::Stash(StashMutation::Apply { .. }) => {
                    "stash applied; it remains available until separately dropped".to_owned()
                }
                GitMutation::Stash(StashMutation::Drop { .. }) => "stash dropped".to_owned(),
                GitMutation::PartialStage(request) if request.scope == DiffScope::Staged => {
                    "hunk unstaged".to_owned()
                }
                GitMutation::PartialStage(_) => "hunk staged".to_owned(),
            });
        let updated_echo = self.update_action_feedback(action, &message);
        if let Some(summary) = producer_summary
            && (!updated_echo || summary.lines().count() > 1)
        {
            self.info_from("Git", "Git operation completed", summary);
        }
        self.status(message);
        if state == GitServiceState::Completed
            && let Some(destination) = created_worktree
        {
            self.attach_created_worktree(destination);
        }
    }

    fn reload_clean_repository_buffers(&mut self) {
        let Some(repository) = self.git.repository().cloned() else {
            return;
        };
        let buffers = self
            .buffers
            .iter()
            .enumerate()
            .filter_map(|(index, buffer)| {
                (!self.closed_buffers.contains(&index)
                    && !buffer.dirty
                    && buffer.kind == BufferKind::File
                    && buffer
                        .path
                        .as_deref()
                        .is_some_and(|path| repository.contains(path) && path.exists()))
                .then_some(index)
            })
            .collect::<Vec<_>>();
        for buffer in buffers {
            let language = buffer_language(&self.buffers[buffer], &self.registry);
            if self.buffers[buffer].reload().is_ok() {
                self.resync_replaced_buffer(buffer, language);
            }
        }
    }

    fn reload_git_paths(&mut self, paths: &[PathBuf]) {
        for path in paths {
            let Some(buffer) = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
                (!self.closed_buffers.contains(&index)
                    && !buffer.is_directory()
                    && buffer.path.as_deref() == Some(path.as_path()))
                .then_some(index)
            }) else {
                continue;
            };
            if self.buffers[buffer].dirty {
                self.error(format!(
                    "Git changed {} on disk, but its unsaved buffer was kept; reload or save it explicitly",
                    path.display()
                ));
                continue;
            }
            let language = buffer_language(&self.buffers[buffer], &self.registry);
            if let Err(error) = self.buffers[buffer].reload() {
                self.error(error.to_string());
            } else {
                self.resync_replaced_buffer(buffer, language);
            }
        }
    }

    /// The tracker and the provider together, as separate borrows.
    ///
    /// Every Git call needs both, and they live in different fields; going
    /// through one accessor keeps that split in one place rather than at each
    /// call site.
    fn git_ports(&mut self) -> Option<(&mut GitTracker, &dyn GitProvider)> {
        let provider = self.ports.git.as_deref()?;
        Some((&mut self.git, provider))
    }

    /// Reads the staged text of one workspace path so its buffer can show
    /// marks.
    ///
    /// Runyte may open an absolute path outside its workspace. That file is
    /// still an ordinary editable buffer, but it must not turn into a Git
    /// target merely because the workspace itself belongs to a repository.
    /// The canonical containment check also keeps an in-workspace symlink to
    /// an external file on the external side of that boundary.
    pub(super) fn track_in_git(&mut self, path: &Path) -> bool {
        let Some(repository) = self.git.repository().cloned() else {
            return false;
        };
        if !self.workspace_contains_path(path) || !repository.contains(path) {
            self.git.forget(path);
            return false;
        }
        if self.ports.git_service.is_some() {
            let _ = self.request_git(GitOperation::StagedContent {
                repository,
                path: path.to_path_buf(),
            });
            return true;
        }
        if let Some((tracker, provider)) = self.git_ports() {
            let _ = tracker.track(provider, path);
        }
        true
    }

    /// Re-reads which files Git considers changed.
    ///
    /// Called after a write, because saving is the one thing the editor does
    /// that changes the answer without changing any buffer.
    pub(super) fn refresh_git_status(&mut self) {
        if self.ports.git_service.is_some() {
            // A bare status carries no line counts, so while the changed-file
            // list is open the whole snapshot is asked for instead: reading
            // the status alone would answer with a list that had just lost its
            // numbers.
            if self.git_status_buffer().is_some() {
                let _ = self.request_git_refresh();
                return;
            }
            let Some(repository) = self.git.repository().cloned() else {
                return;
            };
            let _ = self.request_git(GitOperation::Status { repository });
            return;
        }
        if let Some((tracker, provider)) = self.git_ports() {
            let _ = tracker.refresh_status(provider);
        }
    }

    /// The branch and outstanding-change summary for the status line.
    pub fn git_summary(&self) -> Option<String> {
        let mut summary = match self.git.summary() {
            Some(summary) => summary,
            None if !self.git_state.progress.is_empty() => "git".to_owned(),
            None => return None,
        };
        if self.git_state.snapshot_stale {
            summary.push_str(" · stale");
        }
        if let Some(progress) = self.git_state.progress.values().max_by_key(|progress| {
            (
                progress.state == GitServiceState::Running,
                progress.mutation,
                progress.id,
            )
        }) {
            let elapsed = progress
                .started_at
                .map(|started| format!(" · {}s", started.elapsed().as_secs()))
                .unwrap_or_default();
            summary.push_str(&format!(
                " · {} {} ({}){}{}",
                match progress.state {
                    GitServiceState::Queued => "queued",
                    _ => "running",
                },
                progress.operation,
                progress.repository.display(),
                elapsed,
                if progress.cancellable {
                    " · :git-cancel"
                } else {
                    ""
                }
            ));
        }
        Some(summary)
    }

    fn active_git_mutation(&self) -> Option<&GitServiceProgress> {
        self.git_state
            .progress
            .values()
            .filter(|progress| progress.mutation)
            .max_by_key(|progress| (progress.state == GitServiceState::Running, progress.id))
    }

    /// Whether a frontend should advance its long-running-action animation.
    ///
    /// This is deliberately phrased in UI terms rather than Git terms. Git is
    /// the first producer, but future background services can join the same
    /// presentation contract without changing the event loops or renderer.
    pub fn has_long_running_action(&self) -> bool {
        self.active_git_mutation().is_some()
    }

    pub(crate) fn long_running_action_snapshot(
        &self,
    ) -> Option<crate::snapshot::LongRunningActionSnapshot> {
        let progress = self.active_git_mutation()?;
        let elapsed_millis = progress
            .started_at
            .map_or(0, |started| started.elapsed().as_millis())
            .min(u128::from(u64::MAX)) as u64;
        Some(crate::snapshot::LongRunningActionSnapshot {
            label: format!(
                "Git · {} {}",
                match progress.state {
                    GitServiceState::Queued => "queued",
                    _ => "running",
                },
                progress.operation
            ),
            detail: progress.repository.display().to_string(),
            elapsed_millis,
            cancel_hint: progress.cancellable.then(|| ":git-cancel".to_owned()),
        })
    }

    pub(super) fn cancel_git(&mut self) {
        let Some(progress) = self.git_state.progress.values().max_by_key(|progress| {
            (
                progress.state == GitServiceState::Running,
                progress.mutation,
                progress.id,
            )
        }) else {
            self.status("no Git operation is queued or running");
            return;
        };
        let id = progress.id;
        let operation = progress.operation;
        let mutation = progress.mutation;
        if self
            .ports
            .git_service
            .as_ref()
            .is_some_and(|service| service.cancel(id))
        {
            self.status(if mutation {
                format!(
                    "stopping {operation}; repository state may already have changed and will be refreshed"
                )
            } else {
                format!("stopping {operation}; its result will be discarded")
            });
        } else {
            self.status("the Git operation already finished");
        }
    }

    /// The change mark for one row of one buffer, if it has one.
    pub fn git_change(&self, buffer: usize, row: usize) -> Option<LineChange> {
        self.git
            .change_at(self.buffers[buffer].path.as_deref()?, row)
    }

    /// Brings one buffer's marks up to date with its text.
    ///
    /// Called before every frame, and cheap when nothing has changed: the
    /// buffer's revision settles it without the text being read at all. Only
    /// an edit since the last frame pays for a comparison, and that comparison
    /// runs against the staged text already in memory.
    pub(super) fn update_git_marks(&mut self, buffer_id: usize) {
        let Some(path) = self.buffers[buffer_id].path.clone() else {
            return;
        };
        let revision = self.buffers[buffer_id].revision();
        let Self { git, buffers, .. } = self;
        git.update(&path, revision, || buffers[buffer_id].to_string());
    }

    /// Whether a buffer has a staged text behind it, and so a gutter column.
    pub(super) fn git_tracks(&self, buffer: usize) -> bool {
        self.buffers[buffer]
            .path
            .as_deref()
            .is_some_and(|path| self.git.tracks(path))
    }

    /// The repository and the active buffer's file, or a reported reason why
    /// this command cannot act.
    ///
    /// Every Git command that works on a file needs the same three facts and
    /// fails the same three ways, so the message a person reads comes from one
    /// place rather than four.
    fn git_target(&mut self) -> Option<(Repository, PathBuf)> {
        if !self.has_git() {
            self.error("no `git` executable was found");
            return None;
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return None;
        };
        let Some(path) = self.buffers[self.active().buffer].path.clone() else {
            self.error("this buffer has no file behind it");
            return None;
        };
        if !repository.contains(&path) {
            self.error(format!(
                "{} is outside {}",
                path.display(),
                repository.workdir().display()
            ));
            return None;
        }
        Some((repository, path))
    }

    /// Opens the active file's unstaged diff in a read-only buffer.
    ///
    /// This is Git's patch of what is on disk, which is deliberately not the
    /// same view as the gutter: the gutter follows the buffer as it is typed
    /// into, and a diff of a buffer that has not been written would describe
    /// changes no other tool can see. When they disagree the header says so
    /// rather than leaving the reader to work it out.
    pub(super) fn open_git_diff(&mut self) {
        let Some((repository, path, scope)) = self.git_diff_target() else {
            return;
        };
        if self.ports.git_service.is_some() {
            let _ = self.request_git(GitOperation::Diff {
                repository,
                scope,
                path: Some(path),
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            return;
        };
        let diff = match provider.diff(&repository, scope, Some(&path)) {
            Ok(diff) => diff,
            Err(error) => {
                self.error(error.to_string());
                return;
            }
        };
        self.open_git_diff_result(scope, Some(path), diff);
    }

    pub(super) fn open_git_file_comparison(&mut self) {
        let Some((repository, path, scope)) = self.git_diff_target() else {
            return;
        };
        if self.ports.git_service.is_some() {
            let _ = self.request_git(GitOperation::FileComparison {
                repository,
                scope,
                path,
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            return;
        };
        let comparison = match provider.file_comparison(&repository, scope, &path) {
            Ok(comparison) => comparison,
            Err(error) => {
                self.error(error.to_string());
                return;
            }
        };
        self.open_git_file_comparison_result(scope, path, comparison);
    }

    fn open_git_file_comparison_result(
        &mut self,
        scope: DiffScope,
        path: PathBuf,
        comparison: FileComparison,
    ) {
        let Some(repository) = self.git.repository() else {
            return;
        };
        let relative = repository
            .relative(&path)
            .unwrap_or(&path)
            .display()
            .to_string();
        let text = |content: crate::git::BaseContent| match content {
            crate::git::BaseContent::Absent => Ok(String::new()),
            crate::git::BaseContent::Text(text) => Ok(text),
            crate::git::BaseContent::Binary => Err(()),
        };
        let (Ok(previous), Ok(current)) = (text(comparison.previous), text(comparison.current))
        else {
            self.error(format!(
                "{relative} is binary and cannot be compared as text"
            ));
            return;
        };
        if previous.len() > MAX_DIFF_BYTES || current.len() > MAX_DIFF_BYTES {
            self.error(format!("{relative} is too large to compare"));
            return;
        }

        let (previous_name, current_name) = match scope {
            DiffScope::Staged => (format!("[HEAD {relative}]"), format!("[index {relative}]")),
            DiffScope::Unstaged => (
                format!("[index {relative}]"),
                format!("[worktree {relative}]"),
            ),
        };
        let previous_buffer =
            self.ensure_git_comparison_buffer(&path, scope, true, previous_name, &previous);
        let current_buffer =
            self.ensure_git_comparison_buffer(&path, scope, false, current_name, &current);

        let left_pane = self.active_pane;
        self.push_jump();
        if self.split(Axis::Horizontal, None).is_err() {
            self.error("comparing needs room for two panes");
            return;
        }
        let right_pane = self.active_pane;
        for (pane_id, buffer) in [(left_pane, previous_buffer), (right_pane, current_buffer)] {
            let pane = self.panes.get_mut(&pane_id).expect("split pane exists");
            pane.retarget(buffer);
            pane.replace_selection(Selection::point(0));
            pane.scroll_row = 0;
            pane.scroll_wrap = 0;
            pane.scroll_col = 0;
            pane.folds.clear();
            pane.preserve_scroll = false;
        }
        self.diffs.retain(|session| {
            !session.has_buffer(previous_buffer) && !session.has_buffer(current_buffer)
        });
        self.diffs.push(DiffSession::new(
            DiffSide {
                pane: left_pane,
                buffer: previous_buffer,
            },
            DiffSide {
                pane: right_pane,
                buffer: current_buffer,
            },
            &previous,
            &current,
        ));
        self.status(format!("comparing Git versions of {relative}"));
    }

    fn ensure_git_comparison_buffer(
        &mut self,
        path: &Path,
        scope: DiffScope,
        previous: bool,
        name: String,
        text: &str,
    ) -> usize {
        let identity = GeneratedViewIdentity::GitDiffSide {
            path: path.to_path_buf(),
            scope: format!("{scope:?}"),
            previous,
        };
        if let Some(index) = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index)
                && buffer.generated_view_identity() == Some(&identity))
            .then_some(index)
        }) {
            self.buffers[index].replace_virtual_text(text);
            return index;
        }
        self.buffers
            .push(Buffer::virtual_text_identified(identity, name, text));
        self.syntax.push(None);
        self.buffers.len() - 1
    }

    pub(super) fn open_git_diff_result(
        &mut self,
        scope: DiffScope,
        path: Option<PathBuf>,
        diff: String,
    ) {
        let Some(repository) = self.git.repository().cloned() else {
            return;
        };
        let Some(path) = path else {
            self.open_git_index_result(diff);
            return;
        };
        let (relative, text) = self.git_diff_text(&repository, scope, &path, &diff);
        let identity = GeneratedViewIdentity::GitDiff {
            path: path.clone(),
            scope: format!("{scope:?}"),
        };
        let buffer = self.open_virtual_diff(identity, format!("[git diff {relative}]"), &text);
        self.git_state.diff_buffers.insert(buffer, (path, scope));
        self.remember_git_patch_shape(buffer, &diff);
    }

    fn refresh_git_diff_result(&mut self, scope: DiffScope, path: PathBuf, diff: String) {
        let Some(repository) = self.git.repository().cloned() else {
            return;
        };
        let Some(buffer) = self
            .git_state
            .diff_buffers
            .iter()
            .find_map(|(buffer, value)| {
                (*value == (path.clone(), scope) && !self.closed_buffers.contains(buffer))
                    .then_some(*buffer)
            })
        else {
            return;
        };
        let (_, text) = self.git_diff_text(&repository, scope, &path, &diff);
        self.replace_virtual_preserving_row(buffer, &text);
        self.remember_git_patch_shape(buffer, &diff);
    }

    fn remember_git_patch_shape(&mut self, buffer: usize, diff: &str) {
        match crate::git::parse_hunks(diff.as_bytes()) {
            Ok(hunks) => {
                self.git_state.patch_hunks.insert(buffer, hunks);
                self.git_state.patch_errors.remove(&buffer);
            }
            Err(error) => {
                self.git_state.patch_hunks.remove(&buffer);
                self.git_state
                    .patch_errors
                    .insert(buffer, error.to_string());
            }
        }
    }

    fn git_diff_text(
        &self,
        repository: &Repository,
        scope: DiffScope,
        path: &Path,
        diff: &str,
    ) -> (String, String) {
        let relative = repository
            .relative(path)
            .unwrap_or(path)
            .display()
            .to_string();
        let side = if scope == DiffScope::Staged {
            "staged"
        } else {
            "not staged"
        };
        let mut text = format!("# {side} · {relative}\n");
        if scope == DiffScope::Unstaged && self.has_unsaved_changes(path) {
            text.push_str("# the buffer has unsaved changes; this is the file on disk\n");
        }
        text.push('\n');
        if diff.trim().is_empty() {
            text.push_str(&format!("no {side} changes in {relative}\n"));
        } else {
            text.push_str(diff);
        }
        (relative, text)
    }

    pub(super) fn request_partial_hunk(&mut self, expected_scope: DiffScope, selected_lines: bool) {
        if self.ports.git_service.is_none() {
            self.error("partial staging requires the asynchronous Git service");
            return;
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        let buffer = self.active().buffer;
        let selection = if selected_lines {
            let Some(path) = self.buffers[buffer].path.clone() else {
                self.error("selected-line staging requires a saved file buffer");
                return;
            };
            if self.buffers[buffer].dirty {
                self.error("selected-line staging refuses unsaved buffers; save first");
                return;
            }
            if self.active().selection.len() != 1 {
                self.error("selected-line staging currently accepts one contiguous selection");
                return;
            }
            let range = self.active().selection.primary();
            let first = self.buffers[buffer].offset_to_row(range.from()) + 1;
            let last = self.buffers[buffer]
                .offset_to_row(range.to().min(self.buffers[buffer].len_chars()))
                + 1;
            let guard = BufferRevisionGuard::new();
            self.git_state
                .partial_guards
                .entry(buffer)
                .or_default()
                .push(guard.clone());
            PartialStageSelection {
                path,
                scope: expected_scope,
                buffer: Some((
                    crate::workspace::BufferId::from_index(buffer),
                    crate::workspace::BufferRevision::from_raw(self.buffers[buffer].revision()),
                )),
                guard: Some(guard),
                hunk: None,
                lines: Some((first, last)),
            }
        } else {
            let Some((path, scope)) = self.git_state.diff_buffers.get(&buffer).cloned() else {
                self.error("hunk staging is available in a per-file Git diff view");
                return;
            };
            if scope != expected_scope {
                self.error(if expected_scope == DiffScope::Staged {
                    "this is not a staged diff; use git-stage-hunk"
                } else {
                    "this is a staged diff; use git-unstage-hunk"
                });
                return;
            }
            if self.has_unsaved_changes(&path) {
                self.error("hunk staging refuses unsaved buffers for this path; save first");
                return;
            }
            if let Some(error) = self.git_state.patch_errors.get(&buffer).cloned() {
                self.error(error);
                return;
            }
            let live = self
                .buffers
                .iter()
                .enumerate()
                .find(|(index, candidate)| {
                    !self.closed_buffers.contains(index)
                        && candidate.kind == BufferKind::File
                        && candidate.path.as_deref() == Some(path.as_path())
                })
                .map(|(index, candidate)| (index, candidate.revision()));
            let cursor_row = self.buffers[buffer].offset_to_row(self.active().head());
            let mut hunk_index = None;
            for row in 0..=cursor_row {
                if self.buffers[buffer].line_string(row).starts_with("@@ ") {
                    hunk_index = Some(hunk_index.map_or(0, |index| index + 1));
                }
            }
            let Some(hunk) = hunk_index
                .and_then(|index| self.git_state.patch_hunks.get(&buffer)?.get(index))
                .map(|hunk| hunk.identity.clone())
            else {
                self.error("place the cursor in a text hunk to stage it");
                return;
            };
            let (source, guard) = live.map_or((None, None), |(index, revision)| {
                let guard = BufferRevisionGuard::new();
                self.git_state
                    .partial_guards
                    .entry(index)
                    .or_default()
                    .push(guard.clone());
                (
                    Some((
                        crate::workspace::BufferId::from_index(index),
                        crate::workspace::BufferRevision::from_raw(revision),
                    )),
                    Some(guard),
                )
            });
            PartialStageSelection {
                path,
                scope,
                buffer: source,
                guard,
                hunk: Some(hunk),
                lines: None,
            }
        };
        let tracked_guard = selection
            .buffer
            .zip(selection.guard.as_ref().map(BufferRevisionGuard::id))
            .and_then(|((buffer, _), guard)| buffer.index().map(|index| (index, guard)));
        if self
            .request_git(GitOperation::PreparePartial {
                repository,
                selection: Box::new(selection),
            })
            .is_none()
            && let Some((buffer, guard)) = tracked_guard
        {
            self.forget_partial_guard(buffer, guard, true);
        }
    }

    pub(super) fn invalidate_partial_guards(&mut self, buffer: usize) {
        if let Some(guards) = self.git_state.partial_guards.remove(&buffer) {
            for guard in guards {
                guard.invalidate();
            }
        }
    }

    pub(super) fn invalidate_all_partial_guards(&mut self) {
        for guards in self.git_state.partial_guards.values() {
            for guard in guards {
                guard.invalidate();
            }
        }
        self.git_state.partial_guards.clear();
    }

    fn forget_partial_guard(&mut self, buffer: usize, id: u64, invalidate: bool) {
        let mut empty = false;
        if let Some(guards) = self.git_state.partial_guards.get_mut(&buffer) {
            guards.retain(|guard| {
                if guard.id() == id {
                    if invalidate {
                        guard.invalidate();
                    }
                    false
                } else {
                    true
                }
            });
            empty = guards.is_empty();
        }
        if empty {
            self.git_state.partial_guards.remove(&buffer);
        }
    }

    /// Which file `:git-diff` shows, and which comparison of it.
    ///
    /// In the changed-file list a row already says which side of the index it
    /// stands on, and that side is the change the reader is pointing at: on a
    /// staged row it is what a commit would take, on an unstaged row what it
    /// would not. Anywhere else there is only the file the buffer is showing,
    /// and only one comparison worth offering for it.
    fn git_diff_target(&mut self) -> Option<(Repository, PathBuf, DiffScope)> {
        if !self.active_buffer().is_git_status() {
            let (repository, path) = self.git_target()?;
            return Some((repository, path, DiffScope::Unstaged));
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return None;
        };
        let Some(entry) = self.changed_entry_at_cursor() else {
            self.error("this row is not a file");
            return None;
        };
        let scope = match entry.side {
            crate::git::StatusSide::Staged => DiffScope::Staged,
            crate::git::StatusSide::Unstaged => DiffScope::Unstaged,
        };
        let path = repository.workdir().join(&entry.path);
        Some((repository, path, scope))
    }

    /// Whether any open buffer holds unwritten changes to a path.
    fn has_unsaved_changes(&self, path: &Path) -> bool {
        self.buffers.iter().enumerate().any(|(index, buffer)| {
            !self.closed_buffers.contains(&index)
                && buffer.dirty
                && buffer.path.as_deref() == Some(path)
        })
    }

    /// Opens everything staged for the next commit in a read-only buffer.
    ///
    /// The file list and the patch are one surface on purpose: the question
    /// being asked before a commit is "is this what I meant to record", and
    /// that is answered by seeing which files and what in them at once.
    pub(super) fn open_git_index(&mut self) {
        if !self.has_git() {
            self.error("no `git` executable was found");
            return;
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if self.ports.git_service.is_some() {
            let mut spec = self.git_refresh_spec(&repository);
            spec.staged_diff = true;
            if let Some(id) = self.request_git(GitOperation::Refresh { repository, spec }) {
                self.git_state.index_open_requests.insert(id);
            }
            return;
        }
        self.refresh_git_status();
        let staged = self.git.status().map_or_else(Vec::new, |status| {
            status
                .files
                .iter()
                .filter(|file| file.is_staged())
                .map(|file| {
                    let name = file.original_path.as_ref().map_or_else(
                        || file.path.display().to_string(),
                        |from| format!("{} → {}", from.display(), file.path.display()),
                    );
                    format!("  {} {name}", file.index.marker())
                })
                .collect()
        });
        let Some(provider) = self.ports.git.as_deref() else {
            return;
        };
        let diff = match provider.diff(&repository, DiffScope::Staged, None) {
            Ok(diff) => diff,
            Err(error) => {
                self.error(error.to_string());
                return;
            }
        };

        let mut text = String::new();
        if staged.is_empty() {
            text.push_str("# nothing is staged for commit\n");
        } else {
            text.push_str(&format!(
                "# staged for commit · {} file{}\n\n",
                staged.len(),
                if staged.len() == 1 { "" } else { "s" }
            ));
            for line in &staged {
                text.push_str(line);
                text.push('\n');
            }
        }
        if !diff.trim().is_empty() {
            text.push('\n');
            text.push_str(&diff);
        }
        self.open_virtual_diff(
            GeneratedViewIdentity::GitIndex,
            "[git index]".to_owned(),
            &text,
        );
    }

    pub(super) fn open_git_index_result(&mut self, diff: String) {
        self.update_git_index_result(diff, true);
    }

    pub(super) fn update_git_index_result(&mut self, diff: String, create: bool) {
        let staged = self.git.status().map_or_else(Vec::new, |status| {
            status
                .files
                .iter()
                .filter(|file| file.is_staged())
                .map(|file| {
                    let name = file.original_path.as_ref().map_or_else(
                        || file.path.display().to_string(),
                        |from| format!("{} → {}", from.display(), file.path.display()),
                    );
                    format!("  {} {name}", file.index.marker())
                })
                .collect::<Vec<_>>()
        });
        let mut text = String::new();
        if staged.is_empty() {
            text.push_str("# nothing is staged for commit\n");
        } else {
            text.push_str(&format!(
                "# staged for commit · {} file{}\n\n",
                staged.len(),
                if staged.len() == 1 { "" } else { "s" }
            ));
            for line in staged {
                text.push_str(&line);
                text.push('\n');
            }
        }
        if !diff.trim().is_empty() {
            text.push('\n');
            text.push_str(&diff);
        }
        if let Some(buffer) = self
            .git_state
            .index_buffer
            .filter(|buffer| !self.closed_buffers.contains(buffer))
        {
            self.replace_virtual_preserving_row(buffer, &text);
        } else if create {
            let buffer = self.open_virtual_diff(
                GeneratedViewIdentity::GitIndex,
                "[git index]".to_owned(),
                &text,
            );
            self.git_state.index_buffer = Some(buffer);
        }
    }

    /// Opens the changed-file list, or brings it up to date if it is open.
    ///
    /// The list is where staging stops being one file at a time: rows are
    /// files, and a selection over several of them is a selection of files.
    pub(super) fn open_git_status(&mut self) {
        if !self.has_git() {
            self.error("no `git` executable was found");
            return;
        }
        if self.git.repository().is_none() {
            self.error("this project is not in a Git repository");
            return;
        }
        if let Some(repository) = self.git.repository().cloned() {
            if self.ports.git_service.is_some() {
                // Asked for by hand rather than left to the spec: the list
                // being opened is what makes its line counts worth reading,
                // and the first time it is opened there is no buffer yet for
                // the spec to notice.
                let mut spec = self.git_refresh_spec(&repository);
                spec.stats = true;
                let _ = self.request_git(GitOperation::Refresh { repository, spec });
            } else {
                self.refresh_git_status();
            }
        }
        let text = self.rebuild_git_status_rows();
        let existing = self.git_status_buffer();
        let buffer = match existing {
            Some(existing) => {
                self.buffers[existing].replace_virtual_text(&text);
                existing
            }
            None => {
                self.buffers.push(Buffer::git_status(&text));
                self.syntax.push(None);
                self.buffers.len() - 1
            }
        };
        self.push_jump();
        // On the first file rather than on the heading: the list is opened to
        // act on something, and a caret parked on a title means the first key
        // pressed does nothing.
        let first_file = self
            .git_state
            .status_entries
            .iter()
            .position(Option::is_some)
            .unwrap_or_default();
        let offset = self.buffers[buffer].line_to_offset(first_file);
        let pane = self.active_mut();
        pane.retarget(buffer);
        pane.replace_selection(Selection::point(offset));
        pane.scroll_row = 0;
        pane.scroll_wrap = 0;
        pane.scroll_col = 0;
        self.mode = Mode::Normal;
    }

    /// Opens the local branch list, reusing its one read-only buffer.
    pub(super) fn open_git_branches(&mut self) {
        if !self.has_git() {
            self.error("no `git` executable was found");
            return;
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if self.ports.git_service.is_some() {
            let _ = self.request_git(GitOperation::Branches { repository });
            return;
        }
        let branches = match self
            .ports
            .git
            .as_deref()
            .expect("checked above")
            .branches(&repository)
        {
            Ok(branches) => branches,
            Err(error) => {
                self.error(error.to_string());
                return;
            }
        };
        let text = self.rebuild_git_branch_rows(branches);
        let selected = self
            .git_state
            .branch_rows
            .iter()
            .position(|row| row.branch.as_ref().is_some_and(|branch| branch.current))
            .unwrap_or_default();
        let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_git_branches()).then_some(index)
        });
        let buffer = match existing {
            Some(existing) => {
                self.buffers[existing].replace_virtual_text(&text);
                existing
            }
            None => {
                self.buffers.push(Buffer::git_branches(&text));
                self.syntax.push(None);
                self.buffers.len() - 1
            }
        };
        self.push_jump();
        let offset = self.buffers[buffer].line_to_offset(selected);
        let pane = self.active_mut();
        pane.retarget(buffer);
        pane.replace_selection(Selection::point(offset));
        pane.scroll_row = 0;
        pane.scroll_wrap = 0;
        pane.scroll_col = 0;
        self.mode = Mode::Normal;
    }

    fn open_git_branches_result(&mut self, branches: Vec<Branch>) {
        let text = self.rebuild_git_branch_rows(branches);
        let selected = self
            .git_state
            .branch_rows
            .iter()
            .position(|row| row.branch.as_ref().is_some_and(|branch| branch.current))
            .unwrap_or_default();
        let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_git_branches()).then_some(index)
        });
        let buffer = existing.unwrap_or_else(|| {
            self.buffers.push(Buffer::git_branches(&text));
            self.syntax.push(None);
            self.buffers.len() - 1
        });
        self.buffers[buffer].replace_virtual_text(&text);
        self.push_jump();
        let offset = self.buffers[buffer].line_to_offset(selected);
        let pane = self.active_mut();
        pane.retarget(buffer);
        pane.replace_selection(Selection::point(offset));
        pane.scroll_row = 0;
        pane.scroll_wrap = 0;
        pane.scroll_col = 0;
        self.mode = Mode::Normal;
    }

    pub(super) fn open_git_worktrees(&mut self) {
        if !self.has_git() {
            self.error("no `git` executable was found");
            return;
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if self.ports.git_service.is_some() {
            let _ = self.request_git(GitOperation::Worktrees { repository });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            return;
        };
        match provider.worktrees(&repository) {
            Ok(worktrees) => self.open_git_worktrees_result(worktrees, true),
            Err(error) => self.error(error.to_string()),
        }
    }

    pub(super) fn open_git_worktrees_result(&mut self, worktrees: Vec<Worktree>, activate: bool) {
        let previous = self.selected_worktree_path();
        self.git_state.worktree_rows = worktrees
            .into_iter()
            .map(|worktree| GeneralWorktreeRow { worktree })
            .collect();
        let mut text = String::new();
        for row in &self.git_state.worktree_rows {
            let worktree = &row.worktree;
            let current = worktree.path == self.project_root;
            text.push(if current { '*' } else { ' ' });
            text.push(' ');
            text.push_str(&crate::git::display_path(&worktree.path));
            text.push_str(" · ");
            if worktree.bare {
                text.push_str("bare");
            } else if let Some(branch) = &worktree.branch {
                text.push_str(branch.strip_prefix("refs/heads/").unwrap_or(branch));
            } else if worktree.detached {
                text.push_str("detached");
            } else {
                text.push_str("unknown head");
            }
            if worktree.missing {
                text.push_str(" · missing");
            }
            if worktree.locked.is_some() {
                text.push_str(" · locked");
            }
            if worktree.prunable.is_some() {
                text.push_str(" · prunable");
            }
            text.push('\n');
        }
        let selected = previous
            .as_ref()
            .and_then(|path| {
                self.git_state
                    .worktree_rows
                    .iter()
                    .position(|row| &row.worktree.path == path)
            })
            .or_else(|| {
                self.git_state
                    .worktree_rows
                    .iter()
                    .position(|row| row.worktree.path == self.project_root)
            })
            .unwrap_or_default();
        let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_git_worktrees()).then_some(index)
        });
        let buffer = existing.unwrap_or_else(|| {
            self.buffers.push(Buffer::git_worktrees(&text));
            self.syntax.push(None);
            self.buffers.len() - 1
        });
        self.buffers[buffer].replace_virtual_text(&text);
        if activate || existing.is_some_and(|index| index == self.active().buffer) {
            if activate {
                self.push_jump();
                self.active_mut().retarget(buffer);
            }
            let offset = self.buffers[buffer].line_to_offset(selected);
            let pane = self.active_mut();
            pane.replace_selection(Selection::point(offset));
            pane.scroll_row = 0;
            pane.scroll_wrap = 0;
            pane.scroll_col = 0;
            self.mode = Mode::Normal;
        }
    }

    pub(super) fn selected_worktree_path(&self) -> Option<PathBuf> {
        if !self.active_buffer().is_git_worktrees() {
            return None;
        }
        let row = self.active_buffer().offset_to_row(self.active().head());
        self.git_state
            .worktree_rows
            .get(row)
            .map(|row| row.worktree.path.clone())
    }

    pub(super) fn selected_worktree(&mut self) -> Option<GeneralWorktreeRow> {
        if !self.active_buffer().is_git_worktrees() {
            self.error("worktree actions are only available in the worktree list");
            return None;
        }
        let row = self.active_buffer().offset_to_row(self.active().head());
        let worktree = self.git_state.worktree_rows.get(row).cloned();
        if worktree.is_none() {
            self.error("this row is not a worktree");
        }
        worktree
    }

    pub(super) fn open_selected_worktree(&mut self) {
        let Some(row) = self.selected_worktree() else {
            return;
        };
        if row.worktree.path == self.project_root {
            self.status("this worktree is already open");
            return;
        }
        if row.worktree.bare || row.worktree.missing || row.worktree.prunable.is_some() {
            self.error("this worktree has no usable project directory");
            return;
        }
        if !self.request_workspace_switch(row.worktree.path) {
            return;
        }
        self.should_quit = true;
    }

    /// Asks whether to remove the ordinary worktree on the active row.
    ///
    /// Ownership and structural refusals happen before confirmation. Dirty
    /// state is deliberately left to Git's last-moment check, and removal is
    /// never forced.
    pub(super) fn remove_selected_worktree(&mut self) {
        let Some(row) = self.selected_worktree() else {
            return;
        };
        let worktree = row.worktree;
        if worktree.path == self.project_root {
            self.error("cannot remove the worktree this Runyte workspace is using");
            return;
        }
        if worktree.bare {
            self.error("cannot remove a bare worktree from this view");
            return;
        }
        if let Some(reason) = worktree.locked.as_deref() {
            let detail = if reason.is_empty() {
                String::new()
            } else {
                format!(": {reason}")
            };
            self.error(format!("cannot remove a locked worktree{detail}"));
            return;
        }
        if worktree.missing || worktree.prunable.is_some() {
            self.error(
                "this worktree directory is unavailable; repair or prune it with Git before removing it here",
            );
            return;
        }
        if self.git.repository().is_none() {
            self.error("this project is not in a Git repository");
            return;
        }
        if !self.has_git() {
            self.error("no `git` executable was found");
            return;
        }
        let confirmation = WorktreeRemovalConfirmation {
            path: worktree.path,
            branch: worktree.branch.map(|branch| {
                branch
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&branch)
                    .to_owned()
            }),
        };
        self.status(confirmation.message());
        self.git_worktree_removal = Some(confirmation);
        self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
    }

    pub(super) fn apply_worktree_removal(&mut self, path: PathBuf) {
        let Some(repository) = self.git.repository().cloned() else {
            return;
        };
        if self.ports.git_service.is_some() {
            let mut refresh = self.git_refresh_spec(&repository);
            // Removal changes both the worktree registry and every branch's
            // checkout annotations, even when one of those buffers is hidden.
            refresh.worktrees = true;
            refresh.branches = true;
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation: GitMutation::RemoveWorktree(path),
                refresh,
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            return;
        };
        if let Err(error) = provider.remove_worktree(&repository, &path) {
            self.error(error.to_string());
            return;
        }
        let branches = provider.branches(&repository);
        let worktrees = provider.worktrees(&repository);
        if let Ok(branches) = branches {
            self.refresh_git_branches_from(branches, "");
        }
        if let Ok(worktrees) = worktrees {
            self.open_git_worktrees_result(worktrees, false);
        }
        self.status(format!(
            "removed worktree {}; no branch was deleted",
            crate::git::display_path(&path)
        ));
    }

    pub(super) fn create_worktree_prompt(&mut self, new_branch: bool) {
        let Some(row) = self.selected_worktree() else {
            return;
        };
        let start = row
            .worktree
            .branch
            .as_deref()
            .map(|branch| {
                branch
                    .strip_prefix("refs/heads/")
                    .unwrap_or(branch)
                    .to_owned()
            })
            .or(row.worktree.head);
        let Some(start) = start else {
            self.error("this worktree has no branch or commit to start from");
            return;
        };
        self.git_worktree_start = Some(start);
        self.git_worktree_new_branch = None;
        self.open_prompt(if new_branch {
            PromptKind::NewWorktreeBranch
        } else {
            PromptKind::WorktreeDestination
        });
    }

    pub(super) fn create_worktree(
        &mut self,
        destination: String,
        start: String,
        new_branch: Option<String>,
    ) {
        let destination = destination.trim();
        if destination.is_empty() {
            self.error("a worktree needs an explicit destination");
            return;
        }
        let destination = PathBuf::from(destination);
        let destination = if destination.is_absolute() {
            destination
        } else {
            self.working_directory.join(destination)
        };
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        let request = WorktreeCreate {
            destination,
            start,
            new_branch,
        };
        if self.ports.git_service.is_some() {
            let mut refresh = self.git_refresh_spec(&repository);
            refresh.worktrees = true;
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation: GitMutation::CreateWorktree(request),
                refresh,
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            self.error("no `git` executable was found");
            return;
        };
        match provider.create_worktree(&repository, &request) {
            Ok(()) => {
                let destination = request.destination.clone();
                self.open_git_worktrees();
                self.status(format!("created worktree at {}", destination.display()));
                self.attach_created_worktree(destination);
            }
            Err(error) => self.error(error.to_string()),
        }
    }

    /// Moves a persistent client into a worktree only after Git has
    /// definitively created it. Standalone mode keeps creation as a Git-only
    /// operation and stays in the current workspace.
    fn attach_created_worktree(&mut self, destination: PathBuf) {
        if self.persistent_session && self.request_workspace_switch(destination) {
            self.should_quit = true;
        }
    }

    pub(super) fn open_git_log(&mut self) {
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        let refresh = self.git_refresh_spec(&repository);
        if refresh.log && self.ports.git_service.is_some() {
            let _ = self.request_git(GitOperation::Refresh {
                repository,
                spec: refresh,
            });
            return;
        }
        let request = LogRequest::default();
        if self.ports.git_service.is_some() {
            if let Some(id) = self.request_git(GitOperation::Log {
                repository,
                request,
            }) {
                self.git_state.log_requests.insert(
                    id,
                    LogViewRequest {
                        buffer: None,
                        page: 0,
                    },
                );
            }
        } else if let Some(provider) = self.ports.git.as_deref() {
            match provider.log_page(&repository, &request) {
                Ok(page) => self.open_git_log_result(request, page, 0, !refresh.log),
                Err(error) => self.error(error.to_string()),
            }
        }
    }

    pub(super) fn open_git_commit_search(&mut self) {
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if self.ports.git_service.is_some() {
            let _ = self.request_git(GitOperation::SearchCommits { repository });
        } else if let Some(provider) = self.ports.git.as_deref() {
            match provider.search_commits(&repository) {
                Ok(result) => self.open_git_commit_search_result(result),
                Err(error) => self.error(error.to_string()),
            }
        }
    }

    pub(super) fn open_git_commit_search_result(&mut self, result: CommitSearchResult) {
        let title = if result.limited {
            format!(
                "Git commits · newest {} (limit reached)",
                result.commits.len()
            )
        } else {
            "Git commits".to_owned()
        };
        self.list_actions = result
            .commits
            .iter()
            .map(|commit| ListAction::GitCommit(commit.summary.oid.clone()))
            .collect();
        let items = result
            .commits
            .into_iter()
            .enumerate()
            .map(|(index, commit)| {
                let search = commit.haystack();
                let preview = format!(
                    "{} · {}\n\n{}",
                    commit.summary.author, commit.summary.author_date, commit.message
                );
                PickerItem::searchable(
                    format!("{} {}", commit.summary.abbreviated, commit.summary.subject),
                    "",
                    search,
                    index,
                )
                .with_preview(preview)
            })
            .collect();
        self.list = Some(
            ListPicker::fuzzy(title, items)
                .with_preview("Commit")
                .with_primary_action("open commit"),
        );
    }

    pub(super) fn open_git_stashes(&mut self) {
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        let refresh = self.git_refresh_spec(&repository);
        if self.ports.git_service.is_some() {
            if refresh.stashes {
                let _ = self.request_git(GitOperation::Refresh {
                    repository,
                    spec: refresh,
                });
            } else {
                let _ = self.request_git(GitOperation::Stashes { repository });
            }
        } else if let Some(provider) = self.ports.git.as_deref() {
            match provider.stashes(&repository) {
                Ok(entries) => self.open_git_stashes_result(entries, !refresh.stashes),
                Err(error) => self.error(error.to_string()),
            }
        }
    }

    pub(super) fn open_git_stashes_result(&mut self, entries: Vec<StashEntry>, activate: bool) {
        let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_git_stash()).then_some(index)
        });
        if existing.is_none() && !activate {
            return;
        }
        let previous = existing.map_or_else(Vec::new, |buffer| {
            self.panes
                .iter()
                .filter(|(_, pane)| pane.buffer == buffer)
                .map(|(pane_id, pane)| {
                    let row = self.buffers[buffer].offset_to_row(pane.head());
                    (
                        *pane_id,
                        row,
                        self.git_state
                            .stash_rows
                            .get(row)
                            .map(|entry| entry.oid.clone()),
                    )
                })
                .collect::<Vec<_>>()
        });
        self.git_state.stash_rows = entries;
        let text = self
            .git_state
            .stash_rows
            .iter()
            .map(|entry| {
                format!(
                    "{}  {}  {}",
                    &entry.oid[..12],
                    entry.selector,
                    entry.subject
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let buffer = existing.unwrap_or_else(|| {
            self.buffers.push(Buffer::git_stash(&text));
            self.syntax.push(None);
            self.buffers.len() - 1
        });
        self.buffers[buffer].replace_virtual_text(&text);
        if activate {
            self.push_jump();
            self.active_mut().retarget(buffer);
        }
        for (pane_id, old_row, oid) in previous {
            let row = oid
                .and_then(|oid| {
                    self.git_state
                        .stash_rows
                        .iter()
                        .position(|entry| entry.oid == oid)
                })
                .unwrap_or_else(|| old_row.min(self.git_state.stash_rows.len().saturating_sub(1)));
            let offset = self.buffers[buffer].line_to_offset(row);
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.replace_selection(Selection::point(offset));
            }
        }
        if activate && existing.is_none() {
            self.active_mut().replace_selection(Selection::point(0));
        }
        self.mode = Mode::Normal;
    }

    /// The stash under the cursor, or a refusal naming the way to a cursor
    /// that has one.
    ///
    /// `:git-stash-apply` and `:git-stash-drop` are reachable from the command
    /// palette everywhere, so the buffer they need is the one thing someone
    /// invoking them from the wrong place does not have. Saying only that they
    /// belong to the stash list leaves them to find it; the keys come from the
    /// registry so this cannot outlive the binding it names.
    pub(super) fn selected_stash(&mut self) -> Option<StashEntry> {
        if !self.active_buffer().is_git_stash() {
            let opening = self
                .keymap()
                .global_sequence_for(Mode::Normal, BindingTarget::Colon(ColonCommand::GitStashes))
                .map(|sequence| format!("`:git-stashes` or {sequence}"))
                .unwrap_or_else(|| "`:git-stashes`".to_owned());
            self.error(format!(
                "stash actions are only available in the stash list; open it with {opening}"
            ));
            return None;
        }
        let row = self.active_buffer().offset_to_row(self.active().head());
        let entry = self.git_state.stash_rows.get(row).cloned();
        if entry.is_none() {
            self.error("this row is not a stash");
        }
        entry
    }

    pub(super) fn request_stash_create(&mut self, scope: StashScope, name: Option<String>) {
        let Some(name) = name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
        else {
            self.error("a named stash needs a non-empty name");
            return;
        };
        if name.chars().any(char::is_control) {
            self.error("stash names cannot contain control characters");
            return;
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if !self.repository_buffers_clean(&repository, "create a stash") {
            return;
        }
        let contents = match scope {
            StashScope::TrackedWorktree => {
                "the tracked worktree snapshot (including the index snapshot); staged changes stay applied and untracked files stay"
            }
            StashScope::TrackedWorktreeAndIndex => {
                "tracked worktree and index changes; untracked files stay"
            }
            StashScope::TrackedAndUntracked => "tracked worktree, index, and untracked files",
        };
        let message = format!(
            "Create stash `{name}` containing {contents}?\nEnter confirms.\nEscape cancels."
        );
        self.git_stash_confirmation = Some(GitStashConfirmation {
            repository: repository.clone(),
            mutation: StashMutation::Create { name, scope },
            message: message.clone(),
        });
        self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
        self.status(message);
    }

    pub(super) fn request_selected_stash(&mut self, drop: bool) {
        let Some(entry) = self.selected_stash() else {
            return;
        };
        let Some(repository) = self.git.repository().cloned() else {
            return;
        };
        if !drop && !self.repository_buffers_clean(&repository, "apply a stash") {
            return;
        }
        let message = if drop {
            format!(
                "Drop {} `{}`?\nEnter confirms.\nEscape cancels.",
                entry.selector, entry.subject
            )
        } else {
            format!(
                "Apply {} without dropping it?\nEnter confirms.\nEscape cancels.",
                entry.selector
            )
        };
        self.git_stash_confirmation = Some(GitStashConfirmation {
            repository: repository.clone(),
            mutation: if drop {
                StashMutation::Drop { oid: entry.oid }
            } else {
                StashMutation::Apply { oid: entry.oid }
            },
            message: message.clone(),
        });
        self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
        self.status(message);
    }

    pub(super) fn handle_git_stash_confirmation(&mut self, key: KeyStroke) -> Result<()> {
        match key.code {
            KeyCode::Escape => {
                self.git_stash_confirmation = None;
                self.status("stash action cancelled");
            }
            KeyCode::Enter => {
                let Some(confirmation) = self.git_stash_confirmation.take() else {
                    return Ok(());
                };
                let GitStashConfirmation {
                    repository: expected_repository,
                    mutation,
                    ..
                } = confirmation;
                let Some(repository) = self.git.repository().cloned() else {
                    return Ok(());
                };
                if repository != expected_repository {
                    self.error("stash confirmation belongs to a different repository; retry");
                    return Ok(());
                }
                let action = match mutation {
                    StashMutation::Create { .. } => "create a stash",
                    StashMutation::Apply { .. } => "apply a stash",
                    StashMutation::Drop { .. } => "drop a stash",
                };
                if !matches!(mutation, StashMutation::Drop { .. })
                    && !self.repository_buffers_clean(&repository, action)
                {
                    return Ok(());
                }
                let mut refresh = self.git_refresh_spec(&repository);
                refresh.stashes = true;
                let _ = self.request_git(GitOperation::Mutate {
                    repository,
                    mutation: GitMutation::Stash(mutation),
                    refresh,
                });
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn next_git_log_page(&mut self) {
        let Some(cursor) = self.git_state.log_next.clone() else {
            self.status("this is the last page of the history");
            return;
        };
        self.request_git_log_page(self.git_state.log_page.saturating_add(1), Some(cursor));
    }

    pub(super) fn previous_git_log_page(&mut self) {
        let Some(page) = self.git_state.log_page.checked_sub(1) else {
            self.status("this is the first page of the history");
            return;
        };
        // Page zero has no cursor; every later page kept the one that
        // produced it, so going back re-requests rather than caching commits.
        let cursor = self.git_state.log_cursors.get(page).cloned().flatten();
        self.request_git_log_page(page, cursor);
    }

    fn request_git_log_page(&mut self, page: usize, cursor: Option<LogCursor>) {
        if !self.active_buffer().is_git_log() {
            self.error("log paging is only available in the Git log");
            return;
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        let request = LogRequest {
            cursor,
            ..LogRequest::default()
        };
        if let Some(id) = self.request_git(GitOperation::Log {
            repository,
            request,
        }) {
            self.git_state.log_requests.insert(
                id,
                LogViewRequest {
                    buffer: Some(self.active().buffer),
                    page,
                },
            );
        }
    }

    /// Applies one page of history to the log view.
    ///
    /// A page replaces what the view shows rather than accumulating, so the
    /// page number means the same thing whether it was reached with
    /// `Ctrl-n`, `Ctrl-p`, or a refresh of the page already on screen.
    pub(super) fn open_git_log_result(
        &mut self,
        request: LogRequest,
        page: LogPage,
        target_page: usize,
        activate: bool,
    ) {
        let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_git_log()).then_some(index)
        });
        let pane_selections = existing.map_or_else(Vec::new, |buffer| {
            self.panes
                .iter()
                .filter(|(_, pane)| pane.buffer == buffer)
                .filter_map(|(pane_id, pane)| {
                    let line = self.buffers[buffer].offset_to_row(pane.head());
                    let row = Self::git_log_line_to_row(line)?;
                    self.git_state
                        .log_rows
                        .get(row)
                        .map(|commit| (*pane_id, commit.oid.clone(), line))
                })
                .collect::<Vec<_>>()
        });
        let LogPage {
            commits,
            next,
            total_pages,
        } = page;
        self.git_state.log_rows = commits;
        self.git_state.log_next = next;
        self.git_state.log_page = target_page;
        // Remember the cursor that produced this page so `Ctrl-p` can ask for
        // it again. Pages are only ever reached one step at a time, so the
        // vector stays dense.
        if self.git_state.log_cursors.len() <= target_page {
            self.git_state.log_cursors.resize(target_page + 1, None);
        }
        self.git_state.log_cursors[target_page] = request.cursor.clone();
        self.git_state.log_cursors.truncate(target_page + 1);

        let heading = Self::git_log_page_heading(
            target_page,
            total_pages.max(target_page.saturating_add(1)),
            &self.git_state.log_rows,
        );
        let mut text = heading;
        let mut hints = Vec::new();
        for (row, commit) in self.git_state.log_rows.iter().enumerate() {
            text.push('\n');
            text.push_str(&format!(
                "{}  {}  {}  {}",
                commit.abbreviated, commit.author_date, commit.author, commit.subject
            ));
            if !commit.decorations.is_empty() {
                hints.push((
                    Self::git_log_row_to_line(row),
                    format!("({})", commit.decorations.join(", ")),
                ));
            }
        }
        let buffer = existing.unwrap_or_else(|| {
            self.buffers.push(Buffer::git_log(&text));
            self.syntax.push(None);
            self.buffers.len() - 1
        });
        self.buffers[buffer].replace_git_log_text(&text, hints);
        for (pane_id, oid, previous_row) in pane_selections {
            let selected = self
                .git_state
                .log_rows
                .iter()
                .position(|row| row.oid == oid)
                .map_or_else(
                    || previous_row.min(self.buffers[buffer].len_lines().saturating_sub(1)),
                    Self::git_log_row_to_line,
                );
            let offset = self.buffers[buffer].line_to_offset(selected);
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.replace_selection(Selection::point(offset));
            }
        }
        if activate {
            self.push_jump();
            self.active_mut().retarget(buffer);
            let first_commit = self.buffers[buffer].line_to_offset(1);
            self.active_mut()
                .replace_selection(Selection::point(first_commit));
        }
        if activate || existing.is_some_and(|index| self.active().buffer == index) {
            self.mode = Mode::Normal;
        }
    }

    /// The buffer line showing a given commit row, past the page heading.
    const fn git_log_row_to_line(row: usize) -> usize {
        row + 1
    }

    /// The compact first line of a Git log page.
    ///
    /// Author dates do not necessarily follow topological order, so the span
    /// is calculated over the whole page instead of taking the first and last
    /// rows. The final separator is buffer text; the navigation keys after it
    /// are a read-only row hint supplied by [`Buffer::row_hints`]. Keeping both
    /// parts ASCII-only makes bytes and terminal columns equivalent.
    fn git_log_page_heading(page: usize, total_pages: usize, commits: &[CommitSummary]) -> String {
        let dates = commits.iter().map(|commit| commit.author_date.as_str());
        let oldest = dates.clone().min();
        let newest = dates.max();
        let span = match (oldest, newest) {
            (Some(oldest), Some(newest)) => format!("{oldest} - {newest}"),
            _ => "no commits".to_owned(),
        };
        format!(
            "# page {}/{} | {span} |",
            page.saturating_add(1),
            total_pages.max(1)
        )
    }

    /// The commit row a buffer line shows, or `None` for the page heading.
    pub(super) const fn git_log_line_to_row(line: usize) -> Option<usize> {
        line.checked_sub(1)
    }

    pub(super) fn selected_git_commit_oid(&self) -> Option<String> {
        let row = self.active_buffer().offset_to_row(self.active().head());
        if self.active_buffer().is_git_log() {
            Self::git_log_line_to_row(row)
                .and_then(|row| self.git_state.log_rows.get(row))
                .map(|commit| commit.oid.clone())
        } else if self.active_buffer().is_git_blame() {
            self.git_state
                .blame_rows
                .get(row)
                .and_then(|line| line.oid.clone())
        } else {
            None
        }
    }

    pub(super) fn open_selected_git_commit(&mut self) {
        if !self.active_buffer().is_git_log() && !self.active_buffer().is_git_blame() {
            self.error("commit navigation is only available in log and blame views");
            return;
        }
        let Some(oid) = self.selected_git_commit_oid() else {
            self.error("this row is uncommitted");
            return;
        };
        self.open_git_commit_oid(oid);
    }

    pub(super) fn open_git_commit_oid(&mut self, oid: String) {
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if self.ports.git_service.is_some() {
            let _ = self.request_git(GitOperation::CommitDetail { repository, oid });
        } else if let Some(provider) = self.ports.git.as_deref() {
            match provider.commit_detail(&repository, &oid) {
                Ok(detail) => self.open_git_commit_detail_result(detail),
                Err(error) => self.error(error.to_string()),
            }
        }
    }

    pub(super) fn open_git_commit_detail_result(&mut self, detail: CommitDetail) {
        let oid = detail.summary.oid.clone();
        let name = format!("[git commit {}]", detail.summary.abbreviated);
        let parents = if detail.summary.parents.is_empty() {
            "-".to_owned()
        } else {
            detail.summary.parents.join(" ")
        };
        let mut text = format!(
            "commit {}\nAuthor: {}\nAuthor-time: {}\nParents: {}\n\n{}\n",
            detail.summary.oid,
            detail.summary.author,
            detail.summary.author_time,
            parents,
            detail.body.trim_end()
        );
        let diff_start = text.chars().count();
        text.push_str(&detail.patch);
        let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_git_commit_oid(&oid))
                .then_some(index)
        });
        let buffer = if let Some(existing) = existing {
            self.replace_virtual_preserving_row(existing, &text);
            existing
        } else {
            self.buffers
                .push(Buffer::git_commit(oid, name, &text, diff_start));
            self.syntax.push(None);
            self.buffers.len() - 1
        };
        let already_active = self.active().buffer == buffer;
        self.push_jump();
        self.active_mut().retarget(buffer);
        if !already_active {
            self.active_mut().replace_selection(Selection::point(0));
        }
        self.active_mut().preserve_scroll = false;
        self.mode = Mode::Normal;
    }

    pub(super) fn request_git_blame(&mut self, full_file: bool) {
        let buffer = self.active().buffer;
        let Some(path) = self.buffers[buffer].path.clone() else {
            self.error("Git blame needs a file buffer");
            return;
        };
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if !repository.contains(&path) {
            self.error("this file is outside the current Git working tree");
            return;
        }
        if self.buffers[buffer].len_bytes() > MAX_BLAME_INPUT_BYTES {
            self.error(format!(
                "Git blame accepts buffers up to {MAX_BLAME_INPUT_BYTES} bytes"
            ));
            return;
        }
        if full_file && self.buffers[buffer].len_lines() > MAX_BLAME_LINES {
            self.error(format!(
                "full-file Git blame accepts up to {MAX_BLAME_LINES} lines; use current-line blame"
            ));
            return;
        }
        if self.buffers[buffer]
            .text()
            .rope()
            .chars()
            .any(|character| character == '\0')
        {
            self.error("binary buffers cannot be blamed");
            return;
        }
        let content = self.buffers[buffer].to_string();
        let lines = (!full_file).then(|| {
            let line = self.cursor_position().row + 1;
            (line, line)
        });
        let request = BlameRequest {
            path,
            content,
            lines,
        };
        let source = BlameSource {
            buffer: crate::workspace::BufferId::from_index(buffer),
            revision: crate::workspace::BufferRevision::from_raw(self.buffers[buffer].revision()),
            repository: repository.common_dir().to_path_buf(),
            path: request.path.clone(),
            full_file,
        };
        if self.ports.git_service.is_some() {
            let _ = self.request_git(GitOperation::Blame {
                repository,
                request,
                source,
            });
        } else if let Some(provider) = self.ports.git.as_deref() {
            match provider.blame(&repository, &request) {
                Ok(lines) => self.open_git_blame_result(source, lines),
                Err(error) => self.error(error.to_string()),
            }
        }
    }

    pub(super) fn open_git_blame_result(&mut self, source: BlameSource, lines: Vec<BlameLine>) {
        let Some(buffer) = source.buffer.index() else {
            self.status("discarded stale Git blame for an unknown buffer");
            return;
        };
        if self.closed_buffers.contains(&buffer)
            || self.buffers.get(buffer).is_none_or(|candidate| {
                candidate.revision() != source.revision.get()
                    || candidate.path.as_deref() != Some(source.path.as_path())
                    || self.git.repository().is_none_or(|repository| {
                        repository.common_dir() != source.repository.as_path()
                    })
            })
        {
            self.status("discarded stale Git blame after the buffer changed");
            return;
        }
        if !source.full_file {
            let Some(line) = lines.first() else {
                self.error("Git returned no attribution for this line");
                return;
            };
            let identity = line
                .oid
                .as_deref()
                .map(|oid| &oid[..oid.len().min(12)])
                .unwrap_or("uncommitted");
            self.status(format!("{identity} · {} · {}", line.author, line.summary));
            return;
        }
        self.git_state.blame_rows = lines;
        let text = self
            .git_state
            .blame_rows
            .iter()
            .map(|line| {
                let identity = line
                    .oid
                    .as_deref()
                    .map(|oid| &oid[..oid.len().min(12)])
                    .unwrap_or("uncommitted");
                let date = line.author_date.as_deref().unwrap_or("");
                format!(
                    "{:>6}  {:<12}  {:<10}  {}  {}",
                    line.source_line, identity, date, line.author, line.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_git_blame()).then_some(index)
        });
        let buffer = existing.unwrap_or_else(|| {
            self.buffers.push(Buffer::git_blame(&text));
            self.syntax.push(None);
            self.buffers.len() - 1
        });
        self.buffers[buffer].replace_virtual_text(&text);
        self.push_jump();
        self.active_mut().retarget(buffer);
        self.active_mut().replace_selection(Selection::point(0));
        self.mode = Mode::Normal;
    }

    pub fn take_workspace_switch(&mut self) -> Option<WorkspaceSwitchRequest> {
        let switch = self.workspace_switch.take();
        if switch.is_some() {
            self.should_quit = false;
        }
        switch
    }

    /// Checks out the branch named by the active branch-list row.
    pub(super) fn checkout_selected_branch(&mut self) {
        let Some(branch) = self.selected_branch() else {
            return;
        };
        if branch.current {
            self.status(format!("already on {}", branch.name));
            return;
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if !self.branch_switch_allowed(&repository) {
            return;
        }
        if self.ports.git_service.is_some() {
            let refresh = self.git_refresh_spec(&repository);
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation: GitMutation::Checkout {
                    branch: branch.name,
                },
                refresh,
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            self.error("no `git` executable was found");
            return;
        };
        if let Err(error) = provider.checkout_branch(&repository, &branch.name) {
            self.error(error.to_string());
            return;
        }
        let outcome = format!("checked out {}", branch.name);
        self.finish_branch_switch(&repository, &branch.name, outcome);
    }

    /// The branch the active branch-list row acts on.
    pub(super) fn selected_branch(&mut self) -> Option<Branch> {
        let row = self.active_buffer().offset_to_row(self.active().head());
        let branch = self
            .git_state
            .branch_rows
            .get(row)
            .and_then(|row| row.branch.clone());
        if branch.is_none() {
            self.error("this row is not a branch");
        }
        branch
    }

    /// Whether the editor's own state permits leaving the current branch.
    ///
    /// Git revalidates the index and working tree itself; what only the editor
    /// can see is a buffer holding edits that were never written, which a
    /// checkout would then reload out from under.
    fn branch_switch_allowed(&mut self, repository: &Repository) -> bool {
        self.repository_buffers_clean(repository, "switch branches")
    }

    fn repository_buffers_clean(&mut self, repository: &Repository, action: &str) -> bool {
        if self.buffers.iter().enumerate().any(|(index, buffer)| {
            !self.closed_buffers.contains(&index)
                && buffer.dirty
                && buffer
                    .path
                    .as_deref()
                    .is_some_and(|path| repository.contains(path))
        }) {
            self.error(format!("cannot {action} with unsaved file-buffer changes"));
            return false;
        }
        true
    }

    /// Brings the editor back in step after Git moved the working tree.
    ///
    /// A checkout, a new branch, and a fast-forward all change what is on disk
    /// under open buffers and what the gutter measures against, so all three end
    /// here. `outcome` is the whole success message, because "checked out",
    /// "created", and what a pull did are different sentences rather than one
    /// with a word swapped.
    fn finish_branch_switch(&mut self, repository: &Repository, branch: &str, outcome: String) {
        let mut reload_error = None;
        let buffers = self
            .buffers
            .iter()
            .enumerate()
            .filter_map(|(index, buffer)| {
                (!self.closed_buffers.contains(&index)
                    && buffer.kind == BufferKind::File
                    && buffer
                        .path
                        .as_deref()
                        .is_some_and(|path| repository.contains(path) && path.exists()))
                .then_some(index)
            })
            .collect::<Vec<_>>();
        for buffer in buffers {
            let language_before = buffer_language(&self.buffers[buffer], &self.registry);
            if let Err(error) = self.buffers[buffer].reload() {
                reload_error.get_or_insert_with(|| error.to_string());
                continue;
            }
            self.resync_replaced_buffer(buffer, language_before);
        }

        if let Some((tracker, provider)) = self.git_ports()
            && let Err(error) = tracker.refresh(provider)
        {
            self.error(error.to_string());
            return;
        }
        self.refresh_git_status_buffer();
        self.refresh_git_branches_buffer(branch);
        if let Some(error) = reload_error {
            self.error(format!(
                "{outcome}, but an open buffer could not be reloaded: {error}"
            ));
        } else {
            self.status(outcome);
        }
    }

    /// Asks for the name of a branch to start at the active row's branch.
    ///
    /// The preconditions are checked before the prompt rather than after it:
    /// being told the tree is dirty is worth more before a name has been typed
    /// than after.
    pub(super) fn create_branch_prompt(&mut self) {
        let Some(branch) = self.selected_branch() else {
            return;
        };
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if !self.has_git() {
            self.error("no `git` executable was found");
            return;
        }
        if !self.branch_switch_allowed(&repository) {
            return;
        }
        self.git_branch_start = Some(branch.name);
        self.open_prompt(PromptKind::NewBranch);
    }

    /// Creates the named branch at the row it was started from, and switches.
    pub(super) fn create_branch(&mut self, name: String, start_point: String) {
        let name = name.trim().to_owned();
        if name.is_empty() {
            self.error("a new branch needs a name");
            return;
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if self.ports.git_service.is_some() {
            let refresh = self.git_refresh_spec(&repository);
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation: GitMutation::CreateBranch {
                    branch: name,
                    start: start_point,
                },
                refresh,
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            self.error("no `git` executable was found");
            return;
        };
        if let Err(error) = provider.create_branch(&repository, &name, &start_point) {
            self.error(error.to_string());
            return;
        }
        let outcome = format!("created {name} from {start_point}");
        self.finish_branch_switch(&repository, &name, outcome);
    }

    /// Fast-forwards the current branch onto what it tracks.
    ///
    /// Only the current branch: a pull merges into the working tree, and there
    /// is no working tree for the other rows. In the changed-file list, which
    /// has no branch rows at all, there is nothing else it could mean.
    ///
    /// Production reaches the network on the Git service until the remote
    /// answers or the provider's deadline passes, without holding a frame.
    pub(super) fn pull_current_branch(&mut self) {
        if self.active_buffer().is_git_branches() {
            let Some(branch) = self.selected_branch() else {
                return;
            };
            if !branch.current {
                self.error(format!(
                    "only the current branch can be pulled; check {} out first",
                    branch.name
                ));
                return;
            }
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        // A pull rewrites files under open buffers, and the resync below
        // reloads them, so unwritten edits would be lost rather than merged.
        if self.buffers.iter().enumerate().any(|(index, buffer)| {
            !self.closed_buffers.contains(&index)
                && buffer.dirty
                && buffer
                    .path
                    .as_deref()
                    .is_some_and(|path| repository.contains(path))
        }) {
            self.error("cannot pull with unsaved file-buffer changes");
            return;
        }
        if self.ports.git_service.is_some() {
            let refresh = self.git_refresh_spec(&repository);
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation: GitMutation::Pull,
                refresh,
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            self.error("no `git` executable was found");
            return;
        };
        let summary = match provider.pull(&repository) {
            Ok(summary) => summary,
            Err(error) => {
                self.report_pull_failure(&error);
                return;
            }
        };
        let branch = self
            .git
            .status()
            .map_or_else(String::new, |status| status.head.label());
        // Git's own first line, which says whether anything arrived at all,
        // rather than a sentence Runyte writes over the top of it.
        let outcome = summary
            .lines()
            .find(|line| !line.trim().is_empty())
            .map_or_else(|| format!("pulled {branch}"), |line| line.trim().to_owned());
        self.finish_branch_switch(&repository, &branch, outcome);
    }

    /// Turns a refused pull into either an offer or an error.
    ///
    /// Divergence is the one refusal with a next step, so it opens the
    /// confirmation that carries it out. Everything else is reported the way
    /// every other Git failure is.
    fn report_pull_failure(&mut self, error: &crate::git::GitError) {
        if let crate::git::GitError::Diverged {
            branch,
            upstream,
            ahead,
            behind,
        } = error
        {
            let confirmation = PullRebaseConfirmation {
                branch: branch.clone(),
                upstream: upstream.clone(),
                ahead: *ahead,
                behind: *behind,
            };
            let message = confirmation.message();
            self.git_pull_rebase = Some(confirmation);
            self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
            self.status(message);
            return;
        }
        self.error(error.to_string());
    }

    /// Replays the current branch's unpushed commits onto its upstream, after
    /// a refused pull reported the drift and the reader accepted the offer.
    ///
    /// This rewrites files under open buffers exactly as a pull does, and the
    /// pull that led here already refused unsaved changes, so the reload path
    /// is the same one.
    pub(super) fn rebase_onto_upstream(&mut self) {
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if self.ports.git_service.is_some() {
            let refresh = self.git_refresh_spec(&repository);
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation: GitMutation::RebaseOntoUpstream,
                refresh,
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            self.error("no `git` executable was found");
            return;
        };
        let summary = match provider.rebase_onto_upstream(&repository) {
            Ok(summary) => summary,
            Err(error) => {
                self.error(error.to_string());
                return;
            }
        };
        let branch = self
            .git
            .status()
            .map_or_else(String::new, |status| status.head.label());
        let outcome = summary
            .lines()
            .find(|line| !line.trim().is_empty())
            .map_or_else(
                || format!("replayed {branch}"),
                |line| line.trim().to_owned(),
            );
        self.finish_branch_switch(&repository, &branch, outcome);
    }

    /// Publishes one branch to what it tracks.
    ///
    /// The row's branch in the branch list, the current one in the changed-file
    /// list. Pushing a branch that is not checked out is a real thing to want —
    /// it touches no working tree — so it is not restricted the way pulling is.
    ///
    /// Reaches the network on the same terms as [`App::pull_current_branch`].
    pub(super) fn push_selected_branch(&mut self) {
        let branch = if self.active_buffer().is_git_branches() {
            match self.selected_branch() {
                Some(branch) => branch.name,
                None => return,
            }
        } else {
            match self.git.status().map(|status| &status.head) {
                Some(crate::git::Head::Branch(name)) => name.clone(),
                Some(crate::git::Head::Unborn(_)) => {
                    self.error("this branch has no commits to push yet");
                    return;
                }
                _ => {
                    self.error("HEAD is detached, so there is no branch to push");
                    return;
                }
            }
        };
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if self.ports.git_service.is_some() {
            let refresh = self.git_refresh_spec(&repository);
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation: GitMutation::Push { branch },
                refresh,
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            self.error("no `git` executable was found");
            return;
        };
        let summary = match provider.push(&repository, &branch) {
            Ok(summary) => summary,
            Err(error) => {
                self.error(error.to_string());
                return;
            }
        };
        // Nothing on disk changed, so only what the list says about the branch
        // has to catch up.
        self.refresh_git();
        self.refresh_git_status_buffer();
        self.refresh_git_branches_buffer(&branch);
        if summary.contains("Everything up-to-date") {
            self.status(format!("{branch} was already published"));
        } else {
            self.status(format!("pushed {branch}"));
        }
    }

    /// Asks whether to delete the branch the active branch-list row names.
    pub(super) fn delete_selected_branch(&mut self) {
        let Some(branch) = self.selected_branch() else {
            return;
        };
        if branch.current {
            self.error(
                "cannot delete the branch this working tree is on; check out another branch first",
            );
            return;
        }
        if !branch.checkouts.is_empty() {
            let paths = branch
                .checkouts
                .iter()
                .map(|path| crate::git::display_path(path))
                .collect::<Vec<_>>()
                .join(", ");
            self.error(format!(
                "cannot delete {} because it is checked out at {paths}; remove that worktree from :git-worktrees first",
                branch.name
            ));
            return;
        }
        if self.git.repository().is_none() {
            self.error("this project is not in a Git repository");
            return;
        }
        if !self.has_git() {
            self.error("no `git` executable was found");
            return;
        }
        // Unmerged commits are not lost the way discarded edits are — they stay
        // in the object database, named by the reflog — so the confirmation
        // says what deleting costs rather than calling it irreversible.
        let force = !branch.merged;
        let confirmation = BranchDeletionConfirmation {
            branch: branch.name,
            force,
        };
        let message = confirmation.message();
        self.git_branch_deletion = Some(confirmation);
        self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
        self.status(message);
    }

    /// Removes the confirmed branch and re-reads the list.
    pub(super) fn apply_branch_deletion(&mut self, branch: String, force: bool) {
        let Some(repository) = self.git.repository().cloned() else {
            return;
        };
        if self.ports.git_service.is_some() {
            let refresh = self.git_refresh_spec(&repository);
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation: GitMutation::DeleteBranch { branch, force },
                refresh,
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            return;
        };
        if let Err(error) = provider.delete_branch(&repository, &branch, force) {
            self.error(error.to_string());
            return;
        }
        // The deleted branch is gone from the list, so the caret is asked to
        // follow a name that is not there; the refresh keeps it where it was.
        self.refresh_git_branches_buffer(&branch);
        self.status(format!("deleted {branch}"));
    }

    /// Reprojects the branch list, returning its text.
    ///
    /// The row-to-branch mapping is replaced at the same moment as the text it
    /// describes, for the same reason the changed-file list replaces its own:
    /// a mapping that outlived its rows would delete a branch the reader was
    /// not pointing at.
    fn rebuild_git_branch_rows(&mut self, branches: Vec<Branch>) -> String {
        let rows = crate::git::branch_rows(&branches);
        let text = rows
            .iter()
            .map(|row| row.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.git_state.branch_rows = rows;
        text
    }

    /// Re-reads the open branch list and keeps the caret on `selected`.
    ///
    /// A branch that is no longer there — the one a delete just removed — leaves
    /// the caret on the row that took its place rather than jumping to the top,
    /// so a second delete is aimed where the reader is looking.
    fn refresh_git_branches_buffer(&mut self, selected: &str) {
        let Some(_buffer) = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_git_branches()).then_some(index)
        }) else {
            return;
        };
        let Some(repository) = self.git.repository().cloned() else {
            return;
        };
        let Some(provider) = self.ports.git.as_deref() else {
            return;
        };
        let Ok(branches) = provider.branches(&repository) else {
            return;
        };
        self.refresh_git_branches_from(branches, selected);
    }

    fn refresh_git_branches_from(&mut self, branches: Vec<Branch>, selected: &str) {
        let Some(buffer) = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_git_branches()).then_some(index)
        }) else {
            return;
        };
        let followed = self
            .panes
            .iter()
            .filter(|(_, pane)| pane.buffer == buffer)
            .map(|(pane_id, pane)| {
                let row = self.buffers[buffer].offset_to_row(pane.head());
                let branch = self
                    .git_state
                    .branch_rows
                    .get(row)
                    .and_then(|row| row.branch.as_ref())
                    .map(|branch| branch.name.clone());
                (*pane_id, row, branch)
            })
            .collect::<Vec<_>>();
        let text = self.rebuild_git_branch_rows(branches);
        self.buffers[buffer].replace_virtual_text(&text);
        for (pane_id, previous, branch) in followed {
            let wanted = if selected.is_empty() {
                branch.as_deref()
            } else {
                Some(selected)
            };
            let row = wanted
                .and_then(|wanted| {
                    self.git_state.branch_rows.iter().position(|row| {
                        row.branch
                            .as_ref()
                            .is_some_and(|branch| branch.name == wanted)
                    })
                })
                .unwrap_or_else(|| {
                    previous.min(self.git_state.branch_rows.len().saturating_sub(1))
                });
            let offset = self.buffers[buffer].line_to_offset(row);
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.replace_selection(Selection::point(offset));
            }
        }
    }

    /// Reprojects the list from the current status, returning its text.
    ///
    /// The row-to-file mapping is replaced at the same moment as the text it
    /// describes, because a mapping that outlived its rows would stage a file
    /// the reader was not pointing at.
    fn rebuild_git_status_rows(&mut self) -> String {
        let rows = self.git.status().map_or_else(Vec::new, |status| {
            crate::git::status_rows(status, self.git.stats())
        });
        let text = rows
            .iter()
            .map(|row| row.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.git_state.status_counts = rows.iter().map(|row| row.counts.clone()).collect();
        self.git_state.status_entries = rows.into_iter().map(|row| row.entry).collect();
        text
    }

    /// The one open changed-file list, whether or not a pane is showing it.
    fn git_status_buffer(&self) -> Option<usize> {
        self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_git_status()).then_some(index)
        })
    }

    /// Rewrites an open changed-file list in place, keeping the reader where
    /// they were.
    ///
    /// Staging from the list changes what the list says, and re-opening it
    /// would throw away the row someone had just moved to.
    fn refresh_git_status_buffer(&mut self) {
        let Some(buffer) = self.git_status_buffer() else {
            return;
        };
        // The caret follows the file, not the row number. Staging moves a file
        // into another section, and a caret that stayed where it was would end
        // up on whichever file closed the gap — so the next keypress would act
        // on something nobody chose.
        let followed = self
            .panes
            .iter()
            .filter(|(_, pane)| pane.buffer == buffer)
            .map(|(id, pane)| {
                let row = self.buffers[buffer].offset_to_row(pane.head());
                let path = self
                    .git_state
                    .status_entries
                    .get(row)
                    .and_then(|entry| entry.as_ref())
                    .map(|entry| entry.path.clone());
                (*id, row, path)
            })
            .collect::<Vec<_>>();

        let text = self.rebuild_git_status_rows();
        self.buffers[buffer].replace_virtual_text(&text);

        let rows = self.buffers[buffer].len_lines();
        for (pane_id, previous_row, path) in followed {
            let row = path
                .and_then(|path| {
                    self.git_state
                        .status_entries
                        .iter()
                        .position(|entry| entry.as_ref().is_some_and(|entry| entry.path == path))
                })
                // The file left the list entirely, so its row number is all
                // there is left to keep, and the list may have shrunk under it.
                .unwrap_or_else(|| previous_row.min(rows.saturating_sub(1)));
            let offset = self.buffers[buffer].line_to_offset(row);
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.replace_selection(Selection::point(offset));
            }
        }
    }

    /// The distinct files the selection covers in the changed-file list.
    ///
    /// A file that is both staged and edited again has a row on each side, so
    /// a selection over both must not act on it twice.
    fn selected_changed_files(&self) -> Vec<PathBuf> {
        let pane = self.active();
        let buffer = &self.buffers[pane.buffer];
        let mut paths: Vec<PathBuf> = Vec::new();
        for range in pane.selection.ranges() {
            let first = buffer.offset_to_row(range.from());
            let last = buffer.offset_to_row(range.to().min(buffer.len_chars()));
            for row in first..=last {
                let Some(Some(entry)) = self.git_state.status_entries.get(row) else {
                    continue;
                };
                if !paths.contains(&entry.path) {
                    paths.push(entry.path.clone());
                }
            }
        }
        paths
    }

    /// The row the caret is on in the changed-file list, when it names a file.
    fn changed_entry_at_cursor(&self) -> Option<crate::git::StatusEntry> {
        let pane = self.active();
        let buffer = &self.buffers[pane.buffer];
        let row = buffer.offset_to_row(pane.head());
        self.git_state.status_entries.get(row)?.clone()
    }

    /// Opens the file the caret is on in the changed-file list.
    pub(super) fn open_changed_file(&mut self) -> Result<()> {
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return Ok(());
        };
        let Some(entry) = self.changed_entry_at_cursor() else {
            self.error("this row is not a file");
            return Ok(());
        };
        self.open_file(repository.workdir().join(entry.path))
    }

    /// Stages or unstages every file the selection covers.
    fn stage_selected_files(&mut self, stage: bool) {
        let paths = self.selected_changed_files();
        if paths.is_empty() {
            self.error("no files are selected");
            return;
        }
        self.stage_changed_files(paths, stage);
    }

    /// Stages every row that represents an index-to-worktree change.
    pub(super) fn stage_all_changed_files(&mut self) {
        let mut paths = Vec::new();
        for entry in self.git_state.status_entries.iter().flatten() {
            if entry.side == StatusSide::Unstaged && !paths.contains(&entry.path) {
                paths.push(entry.path.clone());
            }
        }
        if paths.is_empty() {
            self.status("all changed files are already staged");
            return;
        }
        self.stage_changed_files(paths, true);
    }

    fn stage_changed_files(&mut self, paths: Vec<PathBuf>, stage: bool) {
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        if self.ports.git_service.is_some() {
            let paths = paths
                .iter()
                .map(|path| repository.workdir().join(path))
                .collect::<Vec<_>>();
            let refresh = self.git_refresh_spec(&repository);
            let mutation = if stage {
                GitMutation::Stage(paths)
            } else {
                GitMutation::Unstage(paths)
            };
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation,
                refresh,
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            self.error("no `git` executable was found");
            return;
        };
        let mut staged = 0;
        for path in &paths {
            let absolute = repository.workdir().join(path);
            let outcome = if stage {
                provider.stage(&repository, &absolute)
            } else {
                provider.unstage(&repository, &absolute)
            };
            match outcome {
                Ok(()) => staged += 1,
                Err(error) => {
                    self.error(error.to_string());
                    break;
                }
            }
        }
        if staged == 0 {
            return;
        }
        // Each staged file's base moved, and so did the list describing them.
        let tracked = paths
            .iter()
            .take(staged)
            .map(|path| repository.workdir().join(path))
            .collect::<Vec<_>>();
        for path in tracked {
            self.track_in_git(&path);
        }
        self.refresh_git_status();
        self.refresh_git_status_buffer();
        let verb = if stage { "staged" } else { "unstaged" };
        if staged == 1 {
            self.status(format!("{verb} {}", paths[0].display()));
        } else {
            self.status(format!("{verb} {staged} files"));
        }
    }

    /// Asks before throwing a file's uncommitted changes away.
    ///
    /// Everything else in this namespace can be undone by doing the opposite:
    /// unstage what you staged, reset what you committed. Discarded content
    /// was never an object, so nothing in Git will ever produce it again —
    /// which is why this is the one Git command here that asks first.
    pub(super) fn discard_git_changes(&mut self) {
        let Some((repository, paths)) = self.git_discard_targets() else {
            return;
        };
        // Untracked files have no committed version to restore, so discarding
        // one could only mean deleting it. Git keeps that behind `clean` for
        // the same reason, and Runyte keeps it in the explorer, where deleting
        // goes through a confirmed plan and lands in the trash.
        let untracked = self.git.status().map_or_else(Vec::new, |status| {
            status
                .files
                .iter()
                .filter(|file| file.is_untracked())
                .map(|file| file.path.clone())
                .collect()
        });
        let (skipped, tracked): (Vec<_>, Vec<_>) =
            paths.into_iter().partition(|path| untracked.contains(path));
        if tracked.is_empty() {
            self.error(if skipped.is_empty() {
                "nothing here has changes to discard".to_owned()
            } else {
                "untracked files have no committed version to go back to; delete them in the \
                 explorer instead"
                    .to_owned()
            });
            return;
        }

        let unsaved = tracked
            .iter()
            .any(|path| self.has_unsaved_changes(&repository.workdir().join(path)));
        if unsaved {
            self.error(
                "cannot discard while a selected file buffer has unsaved changes; save or discard the buffer first",
            );
            return;
        }
        let confirmation = GitDiscardConfirmation {
            paths: tracked,
            skipped_untracked: skipped.len(),
        };
        let message = confirmation.message();
        self.git_discard_confirmation = Some(confirmation);
        self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
        self.status(message);
    }

    /// The repository-relative paths a discard would act on.
    ///
    /// The selection in the changed-file list, or the file the buffer is
    /// showing — the same rule staging follows, because the reader is pointing
    /// at the same thing either way.
    fn git_discard_targets(&mut self) -> Option<(Repository, Vec<PathBuf>)> {
        if self.active_buffer().is_git_status() {
            let Some(repository) = self.git.repository().cloned() else {
                self.error("this project is not in a Git repository");
                return None;
            };
            let paths = self.selected_changed_files();
            if paths.is_empty() {
                self.error("no files are selected");
                return None;
            }
            return Some((repository, paths));
        }
        let (repository, path) = self.git_target()?;
        let relative = repository.relative(&path)?.to_path_buf();
        Some((repository, vec![relative]))
    }

    /// Restores each path to what `HEAD` holds and reopens what was showing it.
    pub(super) fn apply_git_discard(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        let Some(repository) = self.git.repository().cloned() else {
            return Ok(());
        };
        if self.ports.git_service.is_some() {
            let paths = paths
                .into_iter()
                .map(|path| repository.workdir().join(path))
                .collect::<Vec<_>>();
            let refresh = self.git_refresh_spec(&repository);
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation: GitMutation::Discard(paths),
                refresh,
            });
            return Ok(());
        }
        let Some(provider) = self.ports.git.as_deref() else {
            self.error("no `git` executable was found");
            return Ok(());
        };
        let mut discarded = Vec::new();
        for path in &paths {
            let absolute = repository.workdir().join(path);
            match provider.discard(&repository, &absolute) {
                Ok(()) => discarded.push(absolute),
                Err(error) => {
                    self.error(error.to_string());
                    break;
                }
            }
        }
        if discarded.is_empty() {
            return Ok(());
        }

        // Reopening is required rather than tidy: the file changed underneath
        // its buffer, so the guard that refuses to save over an unexpected
        // disk state would otherwise refuse the next write.
        for absolute in &discarded {
            let open = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
                (!self.closed_buffers.contains(&index)
                    && !buffer.is_directory()
                    && buffer.path.as_deref() == Some(absolute.as_path()))
                .then_some(index)
            });
            let Some(buffer) = open else {
                continue;
            };
            let language_before = buffer_language(&self.buffers[buffer], &self.registry);
            self.buffers[buffer].reload()?;
            self.resync_replaced_buffer(buffer, language_before);
            self.track_in_git(absolute);
        }
        self.refresh_git_status();
        self.refresh_git_status_buffer();

        self.status(match discarded.as_slice() {
            [only] => format!(
                "discarded changes to {}",
                repository
                    .relative(only)
                    .unwrap_or(only.as_path())
                    .display()
            ),
            many => format!("discarded changes to {} files", many.len()),
        });
        Ok(())
    }

    /// Opens a commit message for whatever is staged.
    ///
    /// The template is the one Git hands an external editor: an empty first
    /// line, then commented instructions and the list of files that will be
    /// recorded. Putting them in the buffer rather than in the documentation
    /// means the answer to "what am I about to commit, and how do I finish"
    /// is in front of the person writing the message.
    pub(super) fn open_commit_message(&mut self) {
        if !self.has_git() {
            self.error("no `git` executable was found");
            return;
        }
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        self.refresh_git_status();
        let staged = self.git.status().map_or_else(Vec::new, |status| {
            status
                .files
                .iter()
                .filter(|file| file.is_staged())
                .map(|file| {
                    let name = file.original_path.as_ref().map_or_else(
                        || file.path.display().to_string(),
                        |from| format!("{} → {}", from.display(), file.path.display()),
                    );
                    format!("#   {} {name}", file.index.marker())
                })
                .collect()
        });
        if staged.is_empty() {
            self.error("nothing is staged for commit");
            return;
        }
        let head = self.git.status().map_or_else(
            || repository.workdir().display().to_string(),
            |status| status.head.label(),
        );

        let mut text = String::from("\n");
        text.push_str(&format!(
            "{COMMIT_INSTRUCTIONS}#\n# On {head}\n# Changes to be committed:\n"
        ));
        for line in staged {
            text.push_str(&line);
            text.push('\n');
        }

        let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_commit_message()).then_some(index)
        });
        let buffer = match existing {
            // An abandoned message is replaced rather than resumed: it was
            // written about a different set of staged files.
            Some(existing) => {
                if let Err(error) = self.buffers[existing].discard_changes_to(&text) {
                    self.error(error.to_string());
                    return;
                }
                existing
            }
            None => {
                self.buffers.push(Buffer::commit_message(&text));
                self.syntax.push(None);
                self.buffers.len() - 1
            }
        };
        self.push_jump();
        self.commit_origin = Some(self.active().buffer).filter(|origin| *origin != buffer);
        let pane = self.active_mut();
        pane.retarget(buffer);
        pane.replace_selection(Selection::point(0));
        pane.scroll_row = 0;
        pane.scroll_wrap = 0;
        pane.scroll_col = 0;
        // The first line is where the message goes, and it is empty.
        self.mode = Mode::Insert;
    }

    /// Commits what is staged, using the message in the buffer.
    ///
    /// Reached by writing the message buffer, because writing is already how
    /// this editor says "make this real", and a commit message is the one
    /// piece of text whose write is not to a file.
    pub(super) fn commit_staged(&mut self, buffer_id: usize) {
        let Some(repository) = self.git.repository().cloned() else {
            self.error("this project is not in a Git repository");
            return;
        };
        let message = commit_message_body(&self.buffers[buffer_id].to_string());
        if message.is_empty() {
            self.error("a commit needs a message; write one above the comments");
            return;
        }
        if self.ports.git_service.is_some() {
            let refresh = self.git_refresh_spec(&repository);
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation: GitMutation::Commit { message },
                refresh,
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            self.error("no `git` executable was found");
            return;
        };
        // This fallback exists only for isolated synchronous tests. Production
        // hooks run inside the Git service and never hold the editor frame.
        let summary = match provider.commit(&repository, &message) {
            Ok(summary) => summary,
            // The message stays in the buffer on failure. A rejected hook or
            // an unset identity is something to fix and retry, not a reason to
            // lose what was written.
            Err(error) => {
                self.error(error.to_string());
                return;
            }
        };

        self.refresh_git();
        self.refresh_git_status_buffer();
        // Cleared before closing: a buffer refuses to close while it holds
        // unsaved text, and this text has just been committed.
        let _ = self.buffers[buffer_id].discard_changes_to("");
        self.close_buffer(buffer_id);
        // Back where the detour started — usually the changed-file list, which
        // has just been rewritten to show that the commit took everything.
        self.return_from_commit();
        self.status(
            summary
                .lines()
                .next()
                .unwrap_or("committed")
                .trim()
                .to_owned(),
        );
    }

    /// Abandons a commit message without committing anything.
    ///
    /// Nothing about the index changes: what was staged stays staged, which is
    /// what makes this safe to reach for. Only the text is lost.
    pub(super) fn abandon_commit_message(&mut self, buffer_id: usize) {
        let _ = self.buffers[buffer_id].discard_changes_to("");
        self.close_buffer(buffer_id);
        self.return_from_commit();
        self.status("commit cancelled; nothing was committed and the index is unchanged");
    }

    /// Returns the pane to the buffer a commit message was opened over.
    pub(super) fn return_from_commit(&mut self) {
        let Some(origin) = self
            .commit_origin
            .take()
            .filter(|origin| !self.closed_buffers.contains(origin))
        else {
            return;
        };
        // The list is rebuilt around what the commit took, so its caret goes
        // to the first remaining file rather than to a heading. Any other
        // buffer keeps its own caret, clamped in case it moved.
        let offset = if self.buffers[origin].is_git_status() {
            let row = self
                .git_state
                .status_entries
                .iter()
                .position(Option::is_some)
                .unwrap_or_default();
            self.buffers[origin].line_to_offset(row)
        } else {
            self.panes[&self.active_pane]
                .head()
                .min(self.buffers[origin].len_chars())
        };
        let pane = self.active_mut();
        pane.retarget(origin);
        pane.replace_selection(Selection::point(offset));
    }

    /// Records the active file's disk contents in the index, or takes them
    /// back out again.
    ///
    /// Staging is a file-level operation here: it records what is on disk. A
    /// buffer with unsaved changes is staged anyway rather than refused,
    /// because the alternative is a command that fails for a reason people
    /// have to remember — but the message says which text was recorded, so
    /// nobody has to guess afterwards.
    /// Stages or unstages whatever the reader is pointing at.
    ///
    /// One command rather than two: in the changed-file list that is the
    /// selection, and anywhere else it is the file the buffer is showing. The
    /// key is the same in both places because the intent is.
    pub(super) fn stage_files(&mut self, stage: bool) {
        if self.active_buffer().is_git_status() {
            self.stage_selected_files(stage);
        } else {
            self.stage_active_file(stage);
        }
    }

    fn stage_active_file(&mut self, stage: bool) {
        let Some((repository, path)) = self.git_target() else {
            return;
        };
        if self.ports.git_service.is_some() {
            let refresh = self.git_refresh_spec(&repository);
            let mutation = if stage {
                GitMutation::Stage(vec![path])
            } else {
                GitMutation::Unstage(vec![path])
            };
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation,
                refresh,
            });
            return;
        }
        let Some(provider) = self.ports.git.as_deref() else {
            return;
        };
        let outcome = if stage {
            provider.stage(&repository, &path)
        } else {
            provider.unstage(&repository, &path)
        };
        if let Err(error) = outcome {
            self.error(error.to_string());
            return;
        }
        // The index moved, so both the base every mark is measured against and
        // the counts in the status line are now stale.
        self.track_in_git(&path);
        self.refresh_git_status();

        let relative = repository
            .relative(&path)
            .unwrap_or(&path)
            .display()
            .to_string();
        let verb = if stage { "staged" } else { "unstaged" };
        if stage && self.buffers[self.active().buffer].dirty {
            self.status(format!(
                "{verb} {relative} as written on disk; the buffer has unsaved changes"
            ));
        } else {
            self.status(format!("{verb} {relative}"));
        }
    }

    /// Brings every mark and count back in line with Git.
    ///
    /// This is what an explicit refresh is for: committing, staging, or
    /// switching branches outside the editor moves the text every mark is
    /// measured against, and no buffer changes when it happens.
    pub(super) fn refresh_git(&mut self) {
        if self.ports.git_service.is_some() {
            if self.request_git_refresh() {
                self.status("refreshing Git in the background");
            } else {
                self.error("this project is not in a Git repository");
            }
            return;
        }
        let Some((tracker, provider)) = self.git_ports() else {
            self.error("no `git` executable was found");
            return;
        };
        if tracker.repository().is_none() {
            self.error("this project is not in a Git repository");
            return;
        }
        match tracker.refresh(provider) {
            Ok(()) => {
                // The changed-file list is a projection of what was just
                // re-read, so leaving it as it was would be reporting the
                // refresh and then showing the state before it.
                self.refresh_git_status_buffer();
                let summary = self.git.summary().unwrap_or_default();
                self.status(format!("git: {summary}"));
            }
            Err(error) => self.error(error.to_string()),
        }
    }
}
