// SPDX-License-Identifier: MPL-2.0

//! Native merge review input, details, and bounded semantic snapshots.

use super::git_merges::{ReviewIntent, ReviewPlan};
use super::{App, EditorCommand, KeyCode, KeyStroke, Modifiers};
use crate::git::{BaseContent, GitOperation, GitRequestId, MergePreviewOutcome, RevisionFileView};
use crate::snapshot::{
    OverlayAction, OverlayInput, OverlayKind, OverlayLayout, OverlayPurpose, OverlayRow,
    OverlaySnapshot,
};

pub(super) struct MergeReview {
    pub plan: ReviewPlan,
    pub origin: Option<super::git_merges::ReviewOrigin>,
    pub focus: usize,
    pub root_scroll: usize,
    pub detail: Option<Vec<String>>,
    pub detail_scroll: usize,
    pub acknowledgment: String,
    pub cursor: usize,
    pub requires_ack: bool,
    pub rows: Vec<String>,
    pub title: String,
    pub message: String,
    pub standalone_details: bool,
}

impl MergeReview {
    pub fn new(plan: ReviewPlan, live_terminals: bool) -> Self {
        let (title, mut message, rows) = match &plan {
            ReviewPlan::Sides => ("Inspect conflict sides".into(), String::new(), vec![]),
            ReviewPlan::Merge(p) => {
                let source = p
                    .source_reference
                    .strip_prefix("refs/heads/")
                    .or_else(|| p.source_reference.strip_prefix("refs/remotes/"))
                    .unwrap_or(&p.source_reference);
                let target = p
                    .destination_reference
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&p.destination_reference);
                let prediction = match &p.outcome {
                    MergePreviewOutcome::AlreadyContained => {
                        "Already up to date; approval changes nothing"
                    }
                    MergePreviewOutcome::FastForward => {
                        "Fast-forward; approval moves the current branch, creates no commit"
                    }
                    MergePreviewOutcome::Merge {
                        has_conflicts: true,
                        ..
                    } => "Merge requires conflict resolution and a separate explicit commit",
                    MergePreviewOutcome::Merge { .. } => {
                        "Clean merge; approval leaves the merge awaiting a separate explicit commit"
                    }
                };
                let mut rows = p
                    .comparison
                    .files
                    .iter()
                    .map(|f| {
                        let path = f.right.as_ref().or(f.left.as_ref()).map_or_else(
                            || "non-file change".into(),
                            |p| crate::git::display_path(p),
                        );
                        let status = if f.left.is_none() {
                            "A"
                        } else if f.right.is_none() {
                            "D"
                        } else if f.left != f.right {
                            "R"
                        } else if f.left_mode != f.right_mode {
                            "mode"
                        } else {
                            "M"
                        };
                        let provisional = matches!(&p.outcome, MergePreviewOutcome::Merge { conflicts, .. } if conflicts.iter().any(|c| f.left.as_ref() == Some(&c.path) || f.right.as_ref() == Some(&c.path)));
                        let stats = if provisional { "provisional result; counts omitted".into() } else { f.stats.map_or_else(
                            || "binary or unavailable counts".into(),
                            |s| format!("+{} -{}", s.added, s.removed),
                        ) };
                        format!("{status}  {path}  {stats}")
                    })
                    .collect::<Vec<_>>();
                if rows.is_empty() {
                    rows.push("No changed files".into());
                }
                let mut conflict_count = 0;
                if let MergePreviewOutcome::Merge {
                    conflicts,
                    messages,
                    ..
                } = &p.outcome
                {
                    conflict_count = conflicts.len();
                    for conflict in conflicts {
                        rows.push(format!(
                            "!  {}  {} conflict (provisional)",
                            crate::git::display_path(&conflict.path),
                            conflict.kind()
                        ));
                    }
                    for notice in messages {
                        rows.push(format!("!  {}: {}", notice.kind, notice.message));
                    }
                }
                if conflict_count == 0
                    && !matches!(
                        &p.outcome,
                        MergePreviewOutcome::Merge {
                            has_conflicts: true,
                            ..
                        }
                    )
                {
                    rows.push("No conflicts".into());
                }
                let cached = if p.source_reference.starts_with("refs/remotes/") {
                    " (cached remote tip; no fetch)"
                } else {
                    ""
                };
                (
                    format!("Merge {source} into {target}"),
                    format!(
                        "Current: {target} {} · Other: {source} {}{cached}\n{prediction}\n{} changed files · {conflict_count} provisional conflicts",
                        &p.destination_oid[..8],
                        &p.source_oid[..8],
                        p.comparison.files.len()
                    ),
                    rows,
                )
            }
            ReviewPlan::Resolution(p) => {
                let side = match p.choice {
                    crate::git::ResolutionChoice::SavedFile {
                        allow_literal_markers,
                    } => {
                        if allow_literal_markers {
                            "Stage the entire saved file or reviewed deletion, preserving literal conflict-marker text".into()
                        } else {
                            "Stage the entire saved file or reviewed deletion, including manual edits".into()
                        }
                    }
                    crate::git::ResolutionChoice::Current => {
                        if p.entry.current.is_none() {
                            format!(
                                "Delete the whole path as Current ({}) has no file, then stage the deletion",
                                p.inventory.current_identity
                            )
                        } else {
                            format!(
                                "Replace the entire file with Current ({}), then stage it",
                                p.inventory.current_identity
                            )
                        }
                    }
                    crate::git::ResolutionChoice::Other => {
                        if p.entry.other.is_none() {
                            format!(
                                "Delete the whole path as Other ({}) has no file, then stage the deletion",
                                p.inventory.other_identity
                            )
                        } else {
                            format!(
                                "Replace the entire file with Other ({}), then stage it",
                                p.inventory.other_identity
                            )
                        }
                    }
                };
                let mut rows = vec![format!(
                    "{} · {}",
                    crate::git::display_path(&p.path),
                    if matches!(p.reviewed_content, BaseContent::Absent) {
                        "reviewed deletion"
                    } else {
                        "reviewed complete file"
                    }
                )];
                let related = p.inventory.related_paths(&p.entry.path);
                if related.len() > 1 {
                    rows.push(format!(
                        "Related structural paths: {} · each saved path/deletion must be reviewed",
                        related
                            .iter()
                            .map(|p| crate::git::display_path(p))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                ("Review conflict resolution".into(), side, rows)
            }
            ReviewPlan::Completion(p, intent) => {
                let abort = *intent == ReviewIntent::Abort;
                (if abort { "Abort active merge" } else { "Review merge commit" }.into(), if abort { "Approval aborts this merge. Resolution work and post-merge edits may be discarded; cancellation leaves the merge intact." } else { "Review everything in the index. Approval opens the merge message; only explicitly saving it creates the merge commit." }.into(), if p.staged_diff.is_empty() { vec!["No changed files".into()] } else { p.staged_diff.lines().map(str::to_owned).collect() })
            }
        };
        let requires_ack = live_terminals && matches!(plan, ReviewPlan::Merge(_));
        if requires_ack {
            message.push_str("\nLive terminal sessions keep using this workspace while Git replaces files. Type the exact destination branch to acknowledge them.");
        }
        let focus = rows.len() + usize::from(requires_ack) + 1;
        Self {
            plan,
            origin: None,
            focus,
            root_scroll: 0,
            detail: None,
            detail_scroll: 0,
            acknowledgment: String::new(),
            cursor: 0,
            requires_ack,
            rows,
            title,
            message,
            standalone_details: false,
        }
    }
    pub fn input_focused(&self) -> bool {
        self.requires_ack && self.detail.is_none() && self.focus == self.rows.len()
    }
    fn display_message(&self) -> String {
        let body = self.detail.as_ref().unwrap_or(&self.rows);
        let offset = if self.detail.is_some() {
            self.detail_scroll
        } else {
            self.root_scroll
        };
        if body.len() > offset.saturating_add(510) {
            format!(
                "{}\n{} more rows remain; scroll to review them.",
                self.message,
                body.len() - offset - 510
            )
        } else {
            self.message.clone()
        }
    }
    pub fn approval_available(&self) -> bool {
        self.plan.valid()
            && !self.standalone_details
            && !matches!(&self.plan, ReviewPlan::Merge(p) if matches!(p.outcome, MergePreviewOutcome::AlreadyContained))
            && (!self.requires_ack
                || matches!(&self.plan, ReviewPlan::Merge(p) if self.acknowledgment == p.destination_reference.strip_prefix("refs/heads/").unwrap_or(&p.destination_reference)))
    }
}

impl App {
    fn review_key_hint(&self, command: EditorCommand) -> String {
        let scope = self
            .merge_scope()
            .unwrap_or(crate::keymap::BindingScope::GitMergeReview);
        self.keymap
            .bindings_for_scope(super::Mode::Normal, scope)
            .filter(|binding| {
                binding.scope == scope
                    && binding.target == crate::keymap::BindingTarget::Editor(command)
            })
            .min_by_key(|binding| {
                (
                    usize::from(binding.sequence.as_slice().iter().any(|key| {
                        !matches!(key.code, KeyCode::Char(_))
                            || (!key.modifiers.is_empty() && key.modifiers != Modifiers::SHIFT)
                    })),
                    binding.sequence.len(),
                )
            })
            .map(|binding| binding.sequence.to_string())
            .unwrap_or_default()
    }

    pub(super) fn handle_merge_review_key(&mut self, key: KeyStroke) -> super::Result<()> {
        let scope = self.merge_scope().unwrap();
        if self
            .merge_ui
            .review
            .as_ref()
            .is_some_and(MergeReview::input_focused)
        {
            let review = self.merge_ui.review.as_mut().unwrap();
            match (key.code, key.modifiers) {
                (KeyCode::Char(c), modifiers)
                    if modifiers.is_empty() || modifiers == Modifiers::SHIFT =>
                {
                    super::prompt_insert(&mut review.acknowledgment, review.cursor, c);
                    review.cursor += 1;
                    return Ok(());
                }
                (KeyCode::Backspace, _) => {
                    super::prompt_backspace(&mut review.acknowledgment, &mut review.cursor);
                    return Ok(());
                }
                (KeyCode::Delete, _) => {
                    super::prompt_delete(&mut review.acknowledgment, review.cursor);
                    return Ok(());
                }
                (KeyCode::Left, _) => {
                    review.cursor = review.cursor.saturating_sub(1);
                    return Ok(());
                }
                (KeyCode::Right, _) => {
                    review.cursor = (review.cursor + 1).min(review.acknowledgment.chars().count());
                    return Ok(());
                }
                _ => {}
            }
        }
        if key.code == KeyCode::Escape {
            self.merge_ui.keys.clear();
        }
        self.merge_ui.keys.push(key);
        let command = match self
            .keymap
            .lookup_in(super::Mode::Normal, scope, &self.merge_ui.keys)
        {
            crate::keymap::Lookup::Exact(binding)
            | crate::keymap::Lookup::ExactAndPrefix { exact: binding, .. }
                if binding.scope == scope =>
            {
                let command = match binding.target {
                    crate::keymap::BindingTarget::Editor(c) => Some(c),
                    _ => None,
                };
                self.merge_ui.keys.clear();
                command
            }
            crate::keymap::Lookup::Prefix(_) => None,
            _ => {
                self.merge_ui.keys.clear();
                None
            }
        };
        if let Some(command) = command {
            self.handle_merge_command(command);
        }
        Ok(())
    }

    pub(super) fn reflow_merge_review(&mut self) {
        let Some(review) = self.merge_ui.review.as_mut() else {
            return;
        };
        let area = self.merge_ui.editor_area.unwrap_or(crate::layout::Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        });
        let capacity = usize::from(
            crate::merge_review_layout::merge_review_layout(
                area,
                &review.display_message(),
                review.requires_ack && review.detail.is_none(),
            )
            .body
            .height,
        )
        .clamp(1, 510);
        if review.detail.is_none() && review.focus < review.rows.len() {
            if review.focus < review.root_scroll {
                review.root_scroll = review.focus;
            } else if review.focus >= review.root_scroll.saturating_add(capacity) {
                review.root_scroll = review.focus + 1 - capacity;
            }
        }
    }

    pub(super) fn review_motion(&mut self, command: EditorCommand) {
        use EditorCommand::*;
        let Some(review) = self.merge_ui.review.as_mut() else {
            return;
        };
        if review.detail.is_some() {
            match command {
                MergeReviewBack => {
                    if review.standalone_details {
                        self.cancel_merge_review()
                    } else {
                        review.detail = None;
                        self.merge_ui.details_request = None;
                    }
                }
                MergeReviewEnter => {
                    if review.standalone_details {
                        self.cancel_merge_review()
                    } else {
                        review.detail = None;
                        self.merge_ui.details_request = None;
                    }
                }
                MergeReviewNext => {
                    review.detail_scroll = (review.detail_scroll + 1)
                        .min(review.detail.as_ref().unwrap().len().saturating_sub(1))
                }
                MergeReviewPrevious => {
                    review.detail_scroll = review.detail_scroll.saturating_sub(1)
                }
                _ => {}
            }
            return;
        }
        let approve = review.rows.len() + usize::from(review.requires_ack);
        let cancel = approve + 1;
        match command {
            MergeReviewNext => review.focus = (review.focus + 1).min(cancel),
            MergeReviewPrevious => review.focus = review.focus.saturating_sub(1),
            MergeReviewLeft if review.focus >= approve => review.focus = approve,
            MergeReviewRight if review.focus >= approve => review.focus = cancel,
            MergeReviewEnter if review.focus == approve => self.approve_merge_review(),
            MergeReviewEnter if review.focus == cancel => self.cancel_merge_review(),
            MergeReviewEnter if review.input_focused() => review.focus = approve,
            MergeReviewEnter => self.inspect_merge_change(),
            MergeReviewBack => self.cancel_merge_review(),
            _ => {}
        }
        self.reflow_merge_review();
    }

    fn inspect_merge_change(&mut self) {
        let Some(repository) = self.git.repository().cloned() else {
            return;
        };
        let Some(review) = self.merge_ui.review.as_mut() else {
            return;
        };
        if let ReviewPlan::Resolution(plan) = &review.plan {
            let entry = plan.entry.clone();
            review.detail = Some(vec!["Loading captured index sides…".into()]);
            review.detail_scroll = 0;
            self.merge_ui.details_origin = Some(self.review_origin());
            self.merge_ui.detail_append_result = false;
            self.merge_ui.details_request = self.request_git(GitOperation::ConflictSides {
                repository,
                entry: Box::new(entry),
            });
            return;
        }
        let ReviewPlan::Merge(plan) = &review.plan else {
            return;
        };
        let index = review.focus;
        let conflicts_start = plan.comparison.files.len().max(1);
        let conflict = match &plan.outcome {
            MergePreviewOutcome::Merge { conflicts, .. } => index
                .checked_sub(conflicts_start)
                .and_then(|index| conflicts.get(index))
                .cloned(),
            _ => None,
        };
        let operation = if let Some(entry) = conflict {
            GitOperation::ConflictSides {
                repository,
                entry: Box::new(entry),
            }
        } else if let Some(file) = plan.comparison.files.get(index) {
            GitOperation::RevisionFile {
                repository,
                comparison: Box::new(plan.comparison.endpoints()),
                file: Box::new(file.clone()),
                split: false,
            }
        } else {
            return;
        };
        review.detail = Some(vec!["Loading reviewed change…".into()]);
        review.detail_scroll = 0;
        self.merge_ui.details_origin = Some(self.review_origin());
        self.merge_ui.details_request = self.request_git(operation);
    }

    pub(super) fn receive_merge_detail(
        &mut self,
        request: Option<GitRequestId>,
        view: RevisionFileView,
    ) -> bool {
        if request != self.merge_ui.details_request || request.is_none() {
            return false;
        }
        self.merge_ui.details_request = None;
        if self
            .merge_ui
            .details_origin
            .as_ref()
            .is_none_or(|origin| !origin.matches(self))
        {
            return true;
        }
        if let Some(review) = self.merge_ui.review.as_mut().filter(|r| r.detail.is_some()) {
            let lines = match view {
                RevisionFileView::Patch(text) => {
                    text.lines().map(str::to_owned).collect::<Vec<_>>()
                }
                RevisionFileView::Split(comparison) => {
                    let mut lines = vec![
                        "── Provisional Result (simulated merge; still requires resolution) ──"
                            .into(),
                    ];
                    match comparison.current {
                        BaseContent::Absent => lines.push("Absent in simulated result".into()),
                        BaseContent::Binary => {
                            lines.push("Binary simulated result; text unavailable".into())
                        }
                        BaseContent::Text(text) => lines.extend(text.lines().map(str::to_owned)),
                    }
                    lines
                }
            };
            if self.merge_ui.detail_append_result {
                review.detail.as_mut().unwrap().extend(lines);
                self.merge_ui.detail_append_result = false;
            } else {
                review.detail = Some(lines);
            }
            review.detail_scroll = 0;
        }
        true
    }

    pub(super) fn receive_conflict_sides(
        &mut self,
        request: Option<GitRequestId>,
        entry: crate::git::ConflictEntry,
        sides: [BaseContent; 3],
    ) {
        if request != self.merge_ui.details_request
            || request.is_none()
            || self
                .merge_ui
                .details_origin
                .as_ref()
                .is_none_or(|origin| !origin.matches(self))
        {
            return;
        }
        self.merge_ui.details_request = None;
        let mut lines = vec![crate::git::display_path(&entry.path)];
        let captured_inventory = self
            .merge_ui
            .review
            .as_ref()
            .and_then(|review| match &review.plan {
                ReviewPlan::Resolution(plan) => Some(&plan.inventory),
                ReviewPlan::Completion(plan, _) => Some(&plan.inventory),
                _ => None,
            })
            .or(self.merge_ui.details_inventory.as_ref());
        let labels = captured_inventory.map(|inventory| {
            if matches!(
                inventory.operation,
                crate::git::RepositoryOperation::Idle
                    | crate::git::RepositoryOperation::Merge { .. }
            ) {
                (
                    format!("Current (stage 2): {}", inventory.current_identity),
                    format!("Other (stage 3): {}", inventory.other_identity),
                )
            } else {
                (
                    format!("Stage 2: {}", inventory.current_identity),
                    format!("Stage 3: {}", inventory.other_identity),
                )
            }
        });
        let (current, other) = if let Some(ReviewPlan::Merge(plan)) =
            self.merge_ui.review.as_ref().map(|review| &review.plan)
        {
            (
                format!(
                    "Current (stage 2): {} {}",
                    plan.destination_reference,
                    &plan.destination_oid[..8]
                ),
                format!(
                    "Other (stage 3): {} {}",
                    plan.source_reference,
                    &plan.source_oid[..8]
                ),
            )
        } else {
            labels.unwrap_or_else(|| ("Current (stage 2)".into(), "Other (stage 3)".into()))
        };
        for (name, content) in ["Base (stage 1)".to_owned(), current, other]
            .into_iter()
            .zip(sides)
        {
            lines.push(format!("── {name} ──"));
            match content {
                BaseContent::Absent => lines.push("Absent (deletion or no base)".into()),
                BaseContent::Binary => lines
                    .push("Binary; choose a reviewed whole-file side or resolve externally".into()),
                BaseContent::Text(text) => lines.extend(text.lines().map(str::to_owned)),
            }
        }
        if let Some(review) = self.merge_ui.review.as_mut() {
            if let ReviewPlan::Resolution(plan) = &review.plan {
                lines.push("── Reviewed Result (whole saved file or chosen side) ──".into());
                match &plan.reviewed_content {
                    BaseContent::Absent => {
                        lines.push("Delete this path; reviewed result is absent".into())
                    }
                    BaseContent::Binary => lines.push("Binary result; text unavailable".into()),
                    BaseContent::Text(text) => lines.extend(text.lines().map(str::to_owned)),
                }
            }
            review.detail = Some(lines);
            review.detail_scroll = 0;
        } else if self.merge_ui.inventory.is_some() {
            let mut review = MergeReview::new(ReviewPlan::Sides, false);
            review.title = "Inspect conflict sides".into();
            review.message.clear();
            review.detail = Some(lines);
            review.standalone_details = true;
            review.origin = self.merge_ui.details_origin.clone();
            self.merge_ui.review = Some(review);
        }
        let result_operation = self.merge_ui.review.as_ref().and_then(|review| {
            if let ReviewPlan::Merge(plan) = &review.plan {
                plan.comparison
                    .files
                    .iter()
                    .find(|f| {
                        f.left.as_ref() == Some(&entry.path)
                            || f.right.as_ref() == Some(&entry.path)
                    })
                    .map(|file| GitOperation::RevisionFile {
                        repository: plan.repository.clone(),
                        comparison: Box::new(plan.comparison.endpoints()),
                        file: Box::new(file.clone()),
                        split: true,
                    })
            } else {
                None
            }
        });
        if let Some(operation) = result_operation {
            self.merge_ui.detail_append_result = true;
            self.merge_ui.details_request = self.request_git(operation);
        }
    }

    pub(super) fn merge_review_snapshot(&self) -> Option<OverlaySnapshot> {
        let review = self.merge_ui.review.as_ref()?;
        let detail = review.detail.as_ref();
        let body = detail.unwrap_or(&review.rows);
        let offset = if detail.is_some() {
            review.detail_scroll
        } else {
            review.root_scroll
        };
        let mut rows = body
            .iter()
            .enumerate()
            .skip(offset)
            .take(510)
            .map(|(i, text)| review_row(format!("body:{i}"), text.clone(), true))
            .collect::<Vec<_>>();
        let displayed_body = rows.len();
        rows.push(review_row(
            if detail.is_some() { "back" } else { "approve" },
            if detail.is_some() {
                format!(
                    "[{}] Back",
                    self.review_key_hint(EditorCommand::MergeReviewBack)
                )
            } else {
                let key = self.review_key_hint(EditorCommand::MergeReviewApprove);
                if key.eq_ignore_ascii_case("a") {
                    "[A]pprove merge".into()
                } else {
                    format!("[{key}] Approve merge")
                }
            },
            detail.is_some() || review.approval_available(),
        ));
        let cancel_key = self.review_key_hint(EditorCommand::MergeReviewCancel);
        let cancel_label = if cancel_key.eq_ignore_ascii_case("c") {
            "[C]ancel merge".into()
        } else {
            format!("[{cancel_key}] Cancel merge")
        };
        rows.push(review_row("cancel", cancel_label, true));
        let selected = if detail.is_some() {
            Some(displayed_body)
        } else if review.input_focused() {
            None
        } else if review.focus >= body.len() + usize::from(review.requires_ack) {
            Some(displayed_body + (review.focus - body.len() - usize::from(review.requires_ack)))
        } else {
            review.focus.checked_sub(offset)
        };
        Some(OverlaySnapshot {
            kind: OverlayKind::GitMergeReview,
            purpose: OverlayPurpose::Confirmation,
            input: if review.requires_ack && detail.is_none() {
                OverlayInput::Text
            } else {
                OverlayInput::None
            },
            layout: OverlayLayout::GitMergeReview,
            actions: vec![
                OverlayAction::new(
                    self.review_key_hint(EditorCommand::MergeReviewEnter),
                    "inspect / activate",
                ),
                OverlayAction::new(
                    self.review_key_hint(EditorCommand::MergeReviewCancel),
                    "cancel",
                ),
            ],
            legend: vec![],
            title: review.title.clone(),
            query: review.acknowledgment.clone(),
            query_placeholder: "type exact destination branch".into(),
            column_header: None,
            rows,
            selected,
            scroll_anchor: None,
            row_offset: offset,
            message: Some(review.display_message()),
            omitted_rows: body.len().saturating_sub(offset + 510),
            total_rows: body.len() + 2,
            query_cursor: review.input_focused().then_some(review.cursor),
            show_preview: false,
            preview_title: None,
            preview: None,
        })
    }
}

fn review_row(
    identity: impl Into<String>,
    label: impl Into<String>,
    available: bool,
) -> OverlayRow {
    OverlayRow {
        heading: false,
        identity: identity.into().into(),
        label: label.into(),
        detail: String::new(),
        trailing_detail: String::new(),
        available,
        dimmed: !available,
        muted: vec![],
        emphasis: vec![],
        detail_emphasis: vec![],
        tints: vec![],
        elide_from: None,
    }
}
