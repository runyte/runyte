// SPDX-License-Identifier: MPL-2.0

//! Asynchronous reviewed merging and index-authoritative conflict workflows.

use super::{App, Buffer, EditorCommand, Mode, Selection, Transaction};
use crate::git::{
    self, BufferRevisionGuard, ConflictEntry, ConflictInventory, GitMutation, GitOperation,
    GitRequestId, MergeCompletionPlan, MergePlan, Repository, ResolutionChoice, ResolutionPlan,
};
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct MergeUi {
    pub review: Option<super::git_merge_review::MergeReview>,
    pub editor_area: Option<crate::layout::Rect>,
    pub keys: crate::keymap::KeySequence,
    pub pending: Option<PendingReview>,
    pub inventory: Option<ConflictInventory>,
    pub rows: Vec<Option<PathBuf>>,
    pub conflict_buffer: Option<usize>,
    pub file: Option<PathBuf>,
    pub commit: Option<Box<MergeCompletionPlan>>,
    pub commit_buffer: Option<usize>,
    pub commit_request: Option<GitRequestId>,
    pub guards: Vec<BufferRevisionGuard>,
    pub details_request: Option<GitRequestId>,
    pub details_origin: Option<ReviewOrigin>,
    pub details_inventory: Option<ConflictInventory>,
    pub detail_append_result: bool,
    pub conflict_read: Option<(GitRequestId, ReviewOrigin, bool)>,
    pub mutation_origin: Option<ReviewOrigin>,
    pub commit_preflight: Option<(GitRequestId, ReviewOrigin)>,
}

#[derive(Clone)]
pub(super) struct ReviewOrigin {
    buffer: usize,
    pane: usize,
    revision: u64,
    selection: Selection,
}
impl ReviewOrigin {
    pub(super) fn matches(&self, app: &App) -> bool {
        self.buffer == app.active().buffer
            && self.pane == app.active_pane
            && self.revision == app.active_buffer().revision()
            && self.selection == app.active().selection
    }
}

pub(super) struct PendingReview {
    pub id: GitRequestId,
    pub origin: ReviewOrigin,
    pub guard: BufferRevisionGuard,
    pub intent: ReviewIntent,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ReviewIntent {
    Merge,
    Resolution,
    Continue,
    Abort,
}

pub(super) enum ReviewPlan {
    Sides,
    Merge(Box<MergePlan>),
    Resolution(Box<ResolutionPlan>),
    Completion(Box<MergeCompletionPlan>, ReviewIntent),
}

impl ReviewPlan {
    pub fn invalidate(&self) {
        match self {
            Self::Sides => {}
            Self::Merge(p) => p.invalidate(),
            Self::Resolution(p) => p.invalidate(),
            Self::Completion(p, _) => p.invalidate(),
        }
    }
    pub fn valid(&self) -> bool {
        match self {
            Self::Sides => false,
            Self::Merge(p) => p.guard.is_valid(),
            Self::Resolution(p) => p.guard.is_valid(),
            Self::Completion(p, _) => p.guard.is_valid(),
        }
    }
}

impl App {
    pub(super) fn invalidate_unsubmitted_merge_review(&mut self) {
        if let Some(pending) = self.merge_ui.pending.take() {
            pending.guard.invalidate();
        }
        if let Some(review) = self.merge_ui.review.take() {
            review.plan.invalidate();
        }
        self.merge_ui.details_request = None;
        self.merge_ui.details_origin = None;
        self.merge_ui.details_inventory = None;
        self.merge_ui.conflict_read = None;
        self.merge_ui.commit_preflight = None;
        self.merge_ui.keys.clear();
        if self.merge_ui.commit_request.is_none() {
            if let Some(plan) = self.merge_ui.commit.take() {
                plan.invalidate();
            }
            self.merge_ui.commit_buffer = None;
        }
    }
    pub(super) fn merge_commit_running(&self, buffer: usize) -> bool {
        self.merge_ui.commit_buffer == Some(buffer) && self.merge_ui.commit_request.is_some()
    }
    pub(super) fn refuse_running_merge_message(&mut self, buffer: usize) -> bool {
        if !self.merge_commit_running(buffer) {
            return false;
        }
        self.action_failed(
            "the reviewed merge commit is running; wait for its result before closing the message",
        );
        true
    }
    pub(super) fn close_merge_origin(&mut self, buffer: usize) {
        if self.merge_ui.commit_buffer == Some(buffer) && self.merge_ui.commit_request.is_none() {
            if let Some(plan) = self.merge_ui.commit.take() {
                plan.invalidate();
            }
            self.merge_ui.commit_buffer = None;
        }
        let owns = self
            .merge_ui
            .pending
            .as_ref()
            .is_some_and(|p| p.origin.buffer == buffer)
            || self
                .merge_ui
                .review
                .as_ref()
                .and_then(|r| r.origin.as_ref())
                .is_some_and(|o| o.buffer == buffer);
        if owns {
            self.invalidate_unsubmitted_merge_review();
        }
    }
    pub(super) fn merge_request_failed(&mut self, id: GitRequestId) {
        if self.merge_ui.commit_request == Some(id) {
            self.merge_ui.commit_request = None;
        }
        if self
            .merge_ui
            .commit_preflight
            .as_ref()
            .is_some_and(|(request, _)| *request == id)
        {
            self.merge_ui.commit_preflight = None;
        }
        if self
            .merge_ui
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == id)
        {
            self.merge_ui.pending.take().unwrap().guard.invalidate();
        }
        if self.merge_ui.details_request == Some(id) {
            self.merge_ui.details_request = None;
            if let Some(review) = self.merge_ui.review.as_mut() {
                review.detail = Some(vec![
                    "The reviewed detail could not be read. Back returns to the review.".into(),
                ]);
            }
        }
        if self
            .merge_ui
            .conflict_read
            .as_ref()
            .is_some_and(|(request, _, _)| *request == id)
        {
            self.merge_ui.conflict_read = None;
        }
    }
    pub(super) fn request_commit_operation_preflight(&mut self) {
        if let Some(repository) = self.git.repository().cloned()
            && let Some(id) = self.request_git(GitOperation::Conflicts { repository })
        {
            self.merge_ui.commit_preflight = Some((id, self.review_origin()));
            self.status("checking the repository operation before commit…");
        }
    }
    pub(super) fn refresh_conflict_inventory(&mut self) {
        if self.merge_ui.inventory.as_ref().is_none_or(|i| {
            matches!(i.operation, git::RepositoryOperation::Idle)
                && self.merge_ui.conflict_buffer.is_none()
        }) {
            return;
        }
        if let Some(repository) = self.git.repository().cloned()
            && let Some(id) = self.request_git(GitOperation::Conflicts { repository })
        {
            self.merge_ui.conflict_read = Some((id, self.review_origin(), false));
        }
    }
    pub(super) fn receive_conflict_inventory(
        &mut self,
        request: Option<GitRequestId>,
        inventory: ConflictInventory,
    ) {
        if self
            .merge_ui
            .commit_preflight
            .as_ref()
            .is_some_and(|(id, _)| Some(*id) == request)
        {
            let (_, origin) = self.merge_ui.commit_preflight.take().unwrap();
            if !origin.matches(self) {
                return;
            }
            let active_merge =
                matches!(inventory.operation, git::RepositoryOperation::Merge { .. });
            let ordinary_commit = matches!(inventory.operation, git::RepositoryOperation::Idle);
            self.merge_ui.inventory = Some(inventory);
            if active_merge {
                self.request_merge_completion(ReviewIntent::Continue);
            } else if ordinary_commit {
                self.request_commit_open_refresh();
            } else {
                self.action_failed(
                    "this non-merge Git operation must be continued or aborted with Git",
                );
            }
            return;
        }

        if self
            .merge_ui
            .conflict_read
            .as_ref()
            .is_none_or(|(id, _, _)| Some(*id) != request)
        {
            return;
        }
        let (_, origin, activate) = self.merge_ui.conflict_read.take().unwrap();
        if !origin.matches(self) {
            return;
        }
        self.show_git_conflicts(inventory, activate);
    }
}

impl App {
    pub(super) fn review_origin(&self) -> ReviewOrigin {
        ReviewOrigin {
            buffer: self.active().buffer,
            pane: self.active_pane,
            revision: self.active_buffer().revision(),
            selection: self.active().selection.clone(),
        }
    }
    pub(super) fn merge_scope(&self) -> Option<crate::keymap::BindingScope> {
        use crate::keymap::BindingScope as Scope;
        if let Some(review) = &self.merge_ui.review {
            return Some(if review.detail.is_some() {
                Scope::GitMergeDetail
            } else if review.input_focused() {
                Scope::GitMergeInput
            } else {
                Scope::GitMergeReview
            });
        }
        if self.directory_tree.focused || self.active_terminal().is_some() {
            return None;
        }
        if self.active_buffer().is_git_conflicts() {
            return Some(Scope::GitConflicts);
        }
        if self.selected_conflict().is_some() {
            return Some(if self.is_markdown_document(self.active().buffer) {
                Scope::GitConflictMarkdown
            } else {
                Scope::GitConflictFile
            });
        }
        None
    }

    fn merge_repository(&mut self) -> Option<Repository> {
        let repository = self.git.repository().cloned();
        if repository.is_none() {
            self.action_failed("this project is not in a Git repository");
        }
        repository
    }

    fn merge_unsaved(&self, repository: &Repository) -> bool {
        self.buffers.iter().enumerate().any(|(i, b)| {
            !self.closed_buffers.contains(&i)
                && b.dirty
                && b.path.as_deref().is_some_and(|p| repository.contains(p))
        })
    }

    fn merge_guard(&mut self, repository: &Repository) -> BufferRevisionGuard {
        self.merge_ui
            .guards
            .retain(|guard| guard.is_valid() && guard.has_other_owners());
        let guard = BufferRevisionGuard::new();
        for (i, buffer) in self.buffers.iter_mut().enumerate() {
            if !self.closed_buffers.contains(&i)
                && buffer
                    .path
                    .as_deref()
                    .is_some_and(|p| repository.contains(p))
            {
                buffer.observe_revision(guard.revision_observer());
            }
        }
        self.merge_ui.guards.push(guard.clone());
        guard
    }

    pub(super) fn invalidate_merge_file_guards(&mut self, buffer: usize) {
        if self.buffers[buffer]
            .path
            .as_deref()
            .is_some_and(|p| self.git.repository().is_some_and(|repo| repo.contains(p)))
        {
            for guard in self.merge_ui.guards.drain(..) {
                guard.invalidate();
            }
        }
    }

    pub(super) fn cancel_merge_review(&mut self) {
        if let Some(pending) = self.merge_ui.pending.take() {
            pending.guard.invalidate();
        }
        if let Some(review) = self.merge_ui.review.take() {
            review.plan.invalidate();
        }
        self.merge_ui.details_request = None;
        self.merge_ui.details_origin = None;
        self.merge_ui.details_inventory = None;
        self.merge_ui.keys.clear();
        self.status("Git review cancelled; no reviewed mutation was submitted");
    }

    pub(super) fn prepare_branch_merge(&mut self) {
        let Some(row) = self.selected_branch_row() else {
            return;
        };
        let source = row
            .branch
            .map(|b| format!("refs/heads/{}", b.name))
            .or_else(|| row.remote.map(|r| r.reference));
        let Some(source) = source else { return };
        let Some(repository) = self.merge_repository() else {
            return;
        };
        if self.merge_unsaved(&repository) {
            self.action_failed("save repository file buffers before reviewing a merge");
            return;
        }
        self.cancel_merge_review();
        let guard = self.merge_guard(&repository);
        if let Some(id) = self.request_git(GitOperation::PrepareMerge {
            repository,
            source,
            guard: guard.clone(),
        }) {
            self.merge_ui.pending = Some(PendingReview {
                id,
                origin: self.review_origin(),
                guard,
                intent: ReviewIntent::Merge,
            });
        }
    }

    pub(super) fn request_merge_completion(&mut self, intent: ReviewIntent) {
        let Some(repository) = self.merge_repository() else {
            return;
        };
        if self.merge_unsaved(&repository) {
            self.action_failed(
                "save repository file buffers before continuing or aborting the merge",
            );
            return;
        }
        self.cancel_merge_review();
        let guard = self.merge_guard(&repository);
        if let Some(id) = self.request_git(GitOperation::PrepareMergeCompletion {
            repository,
            guard: guard.clone(),
        }) {
            self.merge_ui.pending = Some(PendingReview {
                id,
                origin: self.review_origin(),
                guard,
                intent,
            });
        }
    }

    pub(super) fn receive_merge_plan(&mut self, request: Option<GitRequestId>, plan: ReviewPlan) {
        if self
            .merge_ui
            .pending
            .as_ref()
            .is_none_or(|pending| request != Some(pending.id))
        {
            plan.invalidate();
            return;
        }
        let Some(pending) = self.merge_ui.pending.take() else {
            plan.invalidate();
            return;
        };
        if request != Some(pending.id)
            || !pending.origin.matches(self)
            || !pending.guard.is_valid()
            || !plan.valid()
        {
            plan.invalidate();
            pending.guard.invalidate();
            return;
        }
        if let ReviewPlan::Completion(p, _) = &plan
            && pending.intent == ReviewIntent::Continue
            && !p.inventory.entries.is_empty()
        {
            plan.invalidate();
            self.action_failed("resolve all index conflicts before continuing the merge");
            return;
        }
        self.mode = Mode::Normal;
        let mut review = super::git_merge_review::MergeReview::new(
            plan,
            self.terminals.iter().any(|t| t.live()),
        );
        review.origin = Some(pending.origin);
        self.merge_ui.review = Some(review);
    }

    pub(super) fn open_git_conflicts(&mut self) {
        if let Some(repository) = self.merge_repository()
            && let Some(id) = self.request_git(GitOperation::Conflicts { repository })
        {
            self.merge_ui.conflict_read = Some((id, self.review_origin(), true));
        }
    }

    pub(super) fn show_git_conflicts(&mut self, inventory: ConflictInventory, activate: bool) {
        let selected = self
            .selected_conflict()
            .and_then(|e| {
                self.git
                    .repository()
                    .map(|repo| repo.workdir().join(e.path))
            })
            .or_else(|| self.merge_ui.file.clone());
        let header = format!(
            "# {:?} · Current: {} · Other: {}",
            inventory.operation, inventory.current_identity, inventory.other_identity
        );
        let mut lines = vec![header, String::new()];
        let mut rows = vec![None, None];
        if inventory.entries.is_empty() {
            lines.push(
                if matches!(inventory.operation, git::RepositoryOperation::Merge { .. }) {
                    "Merge ready to commit"
                } else {
                    "No conflicts"
                }
                .into(),
            );
            rows.push(None);
        }
        for entry in &inventory.entries {
            lines.push(format!(
                "!  {}  {}",
                git::display_path(&entry.path),
                entry.kind()
            ));
            rows.push(Some(entry.path.clone()));
            let related = inventory.related_paths(&entry.path);
            if related.len() > 1 {
                lines.push(format!(
                    "   Related paths: {} · resolve each saved file/deletion",
                    related
                        .iter()
                        .map(|p| git::display_path(p))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                rows.push(None);
            }
        }
        let text = lines.join("\n");
        let existing = self
            .merge_ui
            .conflict_buffer
            .filter(|b| !self.closed_buffers.contains(b));
        let buffer = if let Some(b) = existing {
            self.buffers[b].replace_virtual_text(&text);
            b
        } else {
            self.buffers.push(Buffer::git_conflicts(&text));
            self.syntax.push(None);
            self.buffers.len() - 1
        };
        self.merge_ui.rows = rows;
        self.merge_ui.inventory = Some(inventory);
        self.merge_ui.conflict_buffer = Some(buffer);
        if activate {
            self.push_jump();
            self.active_mut().retarget(buffer);
            self.mode = Mode::Normal;
        }
        if self.active().buffer == buffer {
            let row = selected
                .and_then(|p| {
                    self.git
                        .repository()
                        .and_then(|repo| p.strip_prefix(repo.workdir()).ok().map(PathBuf::from))
                })
                .and_then(|p| {
                    self.merge_ui
                        .rows
                        .iter()
                        .position(|row| row.as_ref() == Some(&p))
                })
                .unwrap_or(2);
            let offset = self.buffers[buffer].line_to_offset(row);
            self.active_mut()
                .replace_selection(Selection::point(offset));
        }
    }

    pub(super) fn selected_conflict(&self) -> Option<ConflictEntry> {
        let inventory = self.merge_ui.inventory.as_ref()?;
        if self.active_buffer().is_git_conflicts() {
            let row = self.active_buffer().offset_to_row(self.active().head());
            let path = self.merge_ui.rows.get(row)?.as_ref()?;
            inventory.entries.iter().find(|e| e.path == *path).cloned()
        } else {
            let path = self.active_buffer().path.as_deref()?;
            let relative = path.strip_prefix(self.git.repository()?.workdir()).ok()?;
            inventory
                .entries
                .iter()
                .find(|e| e.path == relative)
                .cloned()
        }
    }

    pub(super) fn open_git_conflict(&mut self) {
        let Some(entry) = self.selected_conflict() else {
            self.action_failed("this row is not an unresolved conflict");
            return;
        };
        let Some(repository) = self.merge_repository() else {
            return;
        };
        let path = repository.workdir().join(&entry.path);
        self.merge_ui.file = Some(path.clone());
        if !path.exists() {
            self.action_failed(
                "this conflict has no working file; inspect sides or review its deletion",
            );
            return;
        }
        if let Err(error) = self.open_file(path) {
            self.action_failed(error.to_string());
            return;
        }
        self.focus_conflict_region(true, false);
    }

    pub(super) fn focus_conflict_region(&mut self, forward: bool, advance: bool) {
        let Some(entry) = self.selected_conflict() else {
            self.action_failed("this file has no unresolved index stages");
            return;
        };
        let regions = match git::conflict_regions::parse_conflict_regions(
            &self.active_buffer().to_string(),
            entry.marker_width,
        ) {
            Ok(r) => r,
            Err(error) => {
                self.action_failed(error);
                return;
            }
        };
        let head = self.active().head();
        let region = if !advance {
            if forward {
                regions.first()
            } else {
                regions.last()
            }
        } else if forward {
            regions.iter().find(|r| r.range.start > head)
        } else {
            regions.iter().rev().find(|r| r.range.start < head)
        };
        if let Some(region) = region {
            self.active_mut()
                .replace_selection(Selection::point(region.range.start));
            return;
        }
        let Some(inventory) = &self.merge_ui.inventory else {
            return;
        };
        let Some(index) = inventory.entries.iter().position(|e| e.path == entry.path) else {
            return;
        };
        if !advance {
            self.status("this conflict has no recognized text regions; inspect index sides or review a whole-file choice");
            return;
        }
        let next = if forward {
            index + 1
        } else {
            index.wrapping_sub(1)
        };
        if let Some(entry) = inventory.entries.get(next) {
            let path = self.git.repository().unwrap().workdir().join(&entry.path);
            self.merge_ui.file = Some(path.clone());
            if self.open_file(path).is_ok() {
                self.focus_conflict_region(forward, false);
            }
        } else {
            self.status("no further unresolved regions or files");
        }
    }

    pub(super) fn choose_conflict_region(&mut self, other: bool) {
        let Some(entry) = self.selected_conflict() else {
            self.action_failed("this file has no unresolved index stages");
            return;
        };
        let regions = match git::conflict_regions::parse_conflict_regions(
            &self.active_buffer().to_string(),
            entry.marker_width,
        ) {
            Ok(r) => r,
            Err(error) => {
                self.action_failed(error);
                return;
            }
        };
        let head = self.active().head();
        let Some(region) = regions.into_iter().find(|r| r.range.contains(&head)) else {
            self.action_failed("place the caret inside one recognized unresolved region");
            return;
        };
        let side = if other { region.other } else { region.current };
        let content = self.active_buffer().slice(side.start, side.end);
        self.edit(Transaction::change(
            region.range.start,
            region.range.end,
            content,
        ));
    }

    pub(super) fn request_conflict_resolution(&mut self, choice: ResolutionChoice) {
        let Some(entry) = self.selected_conflict() else {
            self.action_failed("this row or file has no unresolved index stages");
            return;
        };
        let Some(repository) = self.merge_repository() else {
            return;
        };
        let path = repository.workdir().join(&entry.path);
        if self.merge_unsaved(&repository) {
            self.action_failed("save repository file buffers before reviewing resolution; this stages the whole file");
            return;
        }
        if !matches!(choice, ResolutionChoice::SavedFile { .. }) {
            let related = self
                .merge_ui
                .inventory
                .as_ref()
                .map(|i| i.related_paths(&entry.path))
                .unwrap_or_default();
            if related.len() > 1 {
                self.action_failed(format!("structural conflict spans {}; edit and save each path, then review its saved file or deletion", related.iter().map(|p| git::display_path(p)).collect::<Vec<_>>().join(", ")));
                return;
            }
        }
        self.cancel_merge_review();
        let guard = self.merge_guard(&repository);
        if let Some(id) = self.request_git(GitOperation::PrepareResolution {
            repository,
            path,
            choice,
            guard: guard.clone(),
        }) {
            self.merge_ui.pending = Some(PendingReview {
                id,
                origin: self.review_origin(),
                guard,
                intent: ReviewIntent::Resolution,
            });
        }
    }

    pub(super) fn inspect_conflict_sides(&mut self) {
        let Some(entry) = self.selected_conflict() else {
            self.action_failed("this row or file has no unresolved index stages");
            return;
        };
        if let Some(repository) = self.merge_repository() {
            self.merge_ui.details_origin = Some(self.review_origin());
            self.merge_ui.details_inventory = self.merge_ui.inventory.clone();
            self.merge_ui.details_request = self.request_git(GitOperation::ConflictSides {
                repository,
                entry: Box::new(entry),
            });
        }
    }

    pub(super) fn return_to_git_conflicts(&mut self) {
        if let Some(buffer) = self
            .merge_ui
            .conflict_buffer
            .filter(|b| !self.closed_buffers.contains(b))
        {
            self.active_mut().retarget(buffer);
            self.mode = Mode::Normal;
        }
        self.open_git_conflicts();
    }

    pub(super) fn approve_merge_review(&mut self) {
        let Some(repository) = self.merge_repository() else {
            return;
        };
        if self.merge_unsaved(&repository) {
            self.action_failed("save repository file buffers and review again");
            return;
        }
        let Some(review) = self.merge_ui.review.as_ref() else {
            return;
        };
        if review
            .origin
            .as_ref()
            .is_none_or(|origin| !origin.matches(self))
        {
            self.cancel_merge_review();
            self.action_failed("the originating view changed; review again");
            return;
        }
        if review.detail.is_some() || !review.approval_available() {
            self.action_failed("the reviewed operation cannot be approved in its current state");
            return;
        }
        let review = self.merge_ui.review.take().unwrap();
        match review.plan {
            ReviewPlan::Completion(plan, ReviewIntent::Continue) => {
                self.merge_ui.commit = Some(plan.clone());
                let text = format!(
                    "{}\n\n# Saving explicitly creates the merge commit.\n# Cancel leaves the merge pending.\n",
                    plan.message.trim_end()
                );
                self.commit_origin = Some(self.active().buffer);
                self.buffers.push(Buffer::commit_message(&text));
                self.syntax.push(None);
                let buffer = self.buffers.len() - 1;
                self.merge_ui.commit_buffer = Some(buffer);
                self.active_mut().retarget(buffer);
                self.mode = Mode::Insert;
            }
            plan => {
                let mutation = match plan {
                    ReviewPlan::Sides => return,
                    ReviewPlan::Merge(p) => GitMutation::Merge(p),
                    ReviewPlan::Resolution(p) => GitMutation::ResolveConflict(p),
                    ReviewPlan::Completion(p, _) => GitMutation::AbortMerge(p),
                };
                self.merge_ui.mutation_origin = Some(self.review_origin());
                let refresh = self.git_refresh_spec(&repository);
                let _ = self.request_git(GitOperation::Mutate {
                    repository,
                    mutation,
                    refresh,
                });
            }
        }
    }

    pub(super) fn commit_reviewed_merge(&mut self, buffer: usize, message: String) -> bool {
        if self.merge_ui.commit_buffer != Some(buffer) {
            return false;
        }
        if self.merge_ui.commit_request.is_some() {
            self.status("the reviewed merge commit is already running");
            return true;
        }
        let Some(plan) = self.merge_ui.commit.as_ref() else {
            return false;
        };
        let Some(repository) = self.git.repository().cloned() else {
            return true;
        };
        if self.merge_unsaved(&repository) {
            self.action_failed("save resolution file buffers before committing the merge");
            return true;
        }
        let refresh = self.git_refresh_spec(&repository);
        self.merge_ui.commit_request = self.request_git(GitOperation::Mutate {
            repository,
            mutation: GitMutation::CommitMerge {
                plan: plan.clone(),
                message,
            },
            refresh,
        });
        true
    }

    pub(super) fn handle_merge_command(&mut self, command: EditorCommand) -> bool {
        use EditorCommand::*;
        match command {
            MergeBranch => self.prepare_branch_merge(),
            OpenGitConflicts => self.open_git_conflicts(),
            OpenGitConflict => self.open_git_conflict(),
            ContinueMerge => self.request_merge_completion(ReviewIntent::Continue),
            AbortMerge => self.request_merge_completion(ReviewIntent::Abort),
            NextConflict => self.focus_conflict_region(true, true),
            PreviousConflict => self.focus_conflict_region(false, true),
            KeepConflictCurrent => self.choose_conflict_region(false),
            TakeConflictOther => self.choose_conflict_region(true),
            InspectConflictSides => self.inspect_conflict_sides(),
            ResolveConflictLiteralMarkers => {
                self.request_conflict_resolution(ResolutionChoice::SavedFile {
                    allow_literal_markers: true,
                })
            }
            ResolveConflict => self.request_conflict_resolution(ResolutionChoice::SavedFile {
                allow_literal_markers: false,
            }),
            KeepConflictFileCurrent => self.request_conflict_resolution(ResolutionChoice::Current),
            TakeConflictFileOther => self.request_conflict_resolution(ResolutionChoice::Other),
            ReturnToGitConflicts => self.return_to_git_conflicts(),
            MergeReviewApprove => self.approve_merge_review(),
            MergeReviewCancel => self.cancel_merge_review(),
            MergeReviewBack | MergeReviewEnter | MergeReviewNext | MergeReviewPrevious
            | MergeReviewLeft | MergeReviewRight => self.review_motion(command),
            _ => return false,
        }
        true
    }
}
