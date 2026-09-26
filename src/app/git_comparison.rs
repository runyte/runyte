// SPDX-License-Identifier: MPL-2.0

//! Native navigation through immutable committed Git comparisons.

use super::view_position::ViewPosition;
use super::*;
use crate::git::{ComparisonTarget, RevisionComparison, RevisionFile, RevisionFileView};

#[derive(Default)]
pub(super) struct ComparisonState {
    documents: HashMap<usize, ComparisonDocument>,
    pending: Option<(GitRequestId, usize, usize, u64)>,
}

struct ComparisonDocument {
    repository: Repository,
    comparison: RevisionComparison,
    width: usize,
}

impl App {
    pub(super) fn open_revision_comparison(&mut self) {
        let Some(repository) = self.git.repository().cloned() else {
            self.action_failed("this project is not in a Git repository");
            return;
        };
        let target = if self.active_buffer().is_git_branches() {
            let Some(row) = self.selected_branch_row() else {
                return;
            };
            if let Some(remote) = row.remote {
                ComparisonTarget::Branch {
                    reference: remote.reference,
                    label: remote.name,
                }
            } else if let Some(branch) = row.branch {
                ComparisonTarget::Branch {
                    reference: format!("refs/heads/{}", branch.name),
                    label: branch.name,
                }
            } else {
                return;
            }
        } else if self.active_buffer().is_git_worktrees() {
            let Some(row) = self.selected_worktree() else {
                return;
            };
            ComparisonTarget::Worktree(row.worktree.path)
        } else {
            self.action_failed("select a branch or worktree to compare");
            return;
        };
        self.request_revision_comparison(repository, target);
    }

    fn request_revision_comparison(&mut self, repository: Repository, target: ComparisonTarget) {
        if self.ports.git_service.is_some() {
            self.queue_revision_operation(GitOperation::CompareRevisions { repository, target });
        } else if let Some(provider) = self.ports.git.as_deref() {
            match provider.compare_revisions(&repository, &target) {
                Ok(comparison) => self.show_revision_comparison(repository, comparison),
                Err(error) => self.error_from("Git", "Comparison failed", error.to_string()),
            }
        }
    }

    fn queue_revision_operation(&mut self, operation: GitOperation) {
        if let Some(id) = self.request_git(operation) {
            self.git_state.comparisons.pending = Some((
                id,
                self.active_pane,
                self.active().buffer,
                self.active().selection_revision,
            ));
        }
    }

    /// Foreground results must not steal focus after navigation or a newer request.
    pub(super) fn accept_revision_response(&mut self, id: GitRequestId) -> bool {
        let Some((request, pane, buffer, revision)) = self.git_state.comparisons.pending else {
            return false;
        };
        if id != request {
            return false;
        }
        self.git_state.comparisons.pending = None;
        self.active_pane == pane
            && self.active().buffer == buffer
            && self.active().selection_revision == revision
            && !self.closed_buffers.contains(&buffer)
    }

    pub(super) fn refresh_revision_comparison(&mut self) -> bool {
        let buffer = self.active().buffer;
        let Some(document) = self.git_state.comparisons.documents.get(&buffer) else {
            return false;
        };
        self.request_revision_comparison(
            document.repository.clone(),
            document.comparison.target.clone(),
        );
        true
    }

    pub(super) fn show_revision_comparison(
        &mut self,
        repository: Repository,
        comparison: RevisionComparison,
    ) {
        let width = self.comparison_width();
        let text = comparison.render(width);
        let identity = GeneratedViewIdentity::GitComparison {
            repository: repository.workdir().to_path_buf(),
            target: comparison.target.clone(),
        };
        let name = format!(
            "[compare {} → {}]",
            comparison.left_label, comparison.right_label
        );
        let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index)
                && buffer.generated_view_identity() == Some(&identity))
            .then_some(index)
        });
        let buffer = if let Some(buffer) = existing {
            self.reproject_revision_list(buffer, &text, Some(&comparison));
            if let BufferKind::Virtual { name: current, .. } = &mut self.buffers[buffer].kind {
                *current = name;
            }
            self.switch_buffer(buffer);
            buffer
        } else {
            self.open_virtual_page(identity, name, &text, ContentAlignment::default())
        };
        self.git_state.comparisons.documents.insert(
            buffer,
            ComparisonDocument {
                repository,
                comparison,
                width,
            },
        );
        if !self.active().saved_view_positions.contains_key(&buffer) {
            let offset = self.buffers[buffer].line_to_offset(4);
            self.active_mut()
                .replace_selection(Selection::point(offset));
            let position = ViewPosition::capture(self.active());
            self.active_mut()
                .saved_view_positions
                .insert(buffer, position);
        }
    }

    /// Rebase both visible selections and return positions before replacing shared text.
    fn reproject_revision_list(
        &mut self,
        buffer: usize,
        text: &str,
        next: Option<&RevisionComparison>,
    ) {
        let old = &self.buffers[buffer];
        let document = self.git_state.comparisons.documents.get(&buffer);
        let rows: HashMap<_, _> = next
            .into_iter()
            .flat_map(|next| next.files.iter().enumerate())
            .map(|(row, file)| ((&file.left, &file.right), row + 4))
            .collect();
        let map_row = |row: usize| {
            document
                .and_then(|document| {
                    row.checked_sub(4)
                        .and_then(|row| document.comparison.files.get(row))
                })
                .and_then(|file| rows.get(&(&file.left, &file.right)).copied())
                .unwrap_or(row)
        };
        let coordinate = |offset: usize| {
            let row = old.offset_to_row(offset.min(old.len_chars()));
            (map_row(row), offset.saturating_sub(old.line_to_offset(row)))
        };
        let positions: Vec<_> = self
            .panes
            .iter()
            .filter_map(|(id, pane)| {
                let mut position = if pane.buffer == buffer {
                    ViewPosition::capture(pane)
                } else {
                    pane.saved_view_positions.get(&buffer)?.clone()
                };
                let ranges: Vec<_> = position
                    .selection
                    .ranges()
                    .iter()
                    .map(|range| (coordinate(range.anchor), coordinate(range.head)))
                    .collect();
                position.scroll_row = map_row(position.scroll_row);
                position.scroll_wrap = 0;
                Some((*id, position, ranges))
            })
            .collect();
        self.buffers[buffer].replace_virtual_text(text);
        let new = &self.buffers[buffer];
        let offset = |(row, column): (usize, usize)| {
            let row = row.min(new.len_lines().saturating_sub(1));
            new.line_to_offset(row) + column.min(new.line_len(row))
        };
        for (id, position, ranges) in positions {
            let position = ViewPosition {
                selection: Selection::new(
                    ranges
                        .into_iter()
                        .map(|(anchor, head)| Range::new(offset(anchor), offset(head)))
                        .collect(),
                    position.selection.primary_index(),
                ),
                scroll_row: position.scroll_row.min(new.len_lines().saturating_sub(1)),
                ..position
            };
            let pane = self.panes.get_mut(&id).unwrap();
            if pane.buffer == buffer {
                position.restore(pane);
            }
            pane.saved_view_positions.insert(buffer, position);
        }
    }

    fn comparison_width(&self) -> usize {
        self.areas
            .get(&self.active_pane)
            .map_or(100, |area| usize::from(area.width.saturating_sub(8)))
    }

    pub(super) fn open_revision_file(&mut self, split: bool) -> bool {
        let buffer = self.active().buffer;
        let Some(document) = self.git_state.comparisons.documents.get(&buffer) else {
            return false;
        };
        let row = self.active_buffer().offset_to_row(self.active().head());
        let Some(file) = row
            .checked_sub(4)
            .and_then(|row| document.comparison.files.get(row))
            .cloned()
        else {
            self.action_failed("this row is not a file");
            return true;
        };
        let repository = document.repository.clone();
        // File reads only need the captured endpoints, not the complete inventory.
        let comparison = document.comparison.endpoints();
        let position = ViewPosition::capture(self.active());
        self.active_mut()
            .saved_view_positions
            .insert(buffer, position);
        if self.ports.git_service.is_some() {
            self.queue_revision_operation(GitOperation::RevisionFile {
                repository,
                comparison: Box::new(comparison),
                file: Box::new(file),
                split,
            });
        } else if let Some(provider) = self.ports.git.as_deref() {
            match provider.revision_file(&repository, &comparison, &file, split) {
                Ok(view) => self.show_revision_file(repository, &comparison, file, view),
                Err(error) => self.error_from("Git", "Comparison failed", error.to_string()),
            }
        }
        true
    }

    pub(super) fn show_revision_file(
        &mut self,
        repository: Repository,
        comparison: &RevisionComparison,
        file: RevisionFile,
        view: RevisionFileView,
    ) {
        let identity = |side| GeneratedViewIdentity::GitRevisionFile {
            repository: repository.workdir().to_path_buf(),
            left: comparison.left_oid.clone(),
            right: comparison.right_oid.clone(),
            file: file.clone(),
            side,
        };
        let path = |path: &Option<PathBuf>| {
            path.as_deref()
                .map(crate::git::display_path)
                .unwrap_or_else(|| "absent".into())
        };
        match view {
            RevisionFileView::Patch(patch) => {
                let text = format!(
                    "# {} ({}) → {} ({}) · committed changes\n\n{patch}",
                    comparison.left_label,
                    &comparison.left_oid[..8],
                    comparison.right_label,
                    &comparison.right_oid[..8]
                );
                self.open_virtual_diff(
                    identity(None),
                    format!("[diff {} → {}]", path(&file.left), path(&file.right)),
                    &text,
                );
            }
            RevisionFileView::Split(contents) => {
                let text = |content| match content {
                    crate::git::BaseContent::Absent => Some(String::new()),
                    crate::git::BaseContent::Text(text) => Some(text),
                    crate::git::BaseContent::Binary => None,
                };
                let (Some(left), Some(right)) = (text(contents.previous), text(contents.current))
                else {
                    self.action_failed("this file is binary and cannot be compared as text");
                    return;
                };
                if left.len() > MAX_DIFF_BYTES || right.len() > MAX_DIFF_BYTES {
                    self.action_failed("this file is too large to compare");
                    return;
                }
                let return_buffer = self.active().buffer;
                let left_pane = self.active_pane;
                self.push_jump();
                if self.split(Axis::Horizontal, None).is_err() {
                    self.action_failed("comparing needs room for two panes");
                    return;
                }
                let right_pane = self.active_pane;
                let mut buffers = Vec::new();
                for (side, label, oid, path, text) in [
                    (
                        true,
                        &comparison.left_label,
                        &comparison.left_oid,
                        &file.left,
                        &left,
                    ),
                    (
                        false,
                        &comparison.right_label,
                        &comparison.right_oid,
                        &file.right,
                        &right,
                    ),
                ] {
                    let identity = identity(Some(side));
                    let existing = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
                        (!self.closed_buffers.contains(&index)
                            && buffer.generated_view_identity() == Some(&identity)
                            && !self.panes.values().any(|pane| pane.buffer == index)
                            && !self.diffs.iter().any(|diff| diff.has_buffer(index)))
                        .then_some(index)
                    });
                    let buffer = existing.unwrap_or_else(|| {
                        self.buffers.push(Buffer::virtual_text_identified(
                            identity,
                            format!("[{label} {} {}]", &oid[..8], display_revision_path(path)),
                            text,
                        ));
                        self.syntax.push(None);
                        self.buffers.len() - 1
                    });
                    buffers.push(buffer);
                }
                for (pane_id, buffer) in [(left_pane, buffers[0]), (right_pane, buffers[1])] {
                    let pane = self.panes.get_mut(&pane_id).unwrap();
                    pane.retarget(buffer);
                    pane.replace_selection(Selection::point(0));
                    pane.scroll_row = 0;
                    pane.scroll_wrap = 0;
                    pane.scroll_col = 0;
                    pane.folds.clear();
                    pane.preserve_scroll = false;
                }
                self.diffs.push(
                    DiffSession::new(
                        DiffSide {
                            pane: left_pane,
                            buffer: buffers[0],
                        },
                        DiffSide {
                            pane: right_pane,
                            buffer: buffers[1],
                        },
                        &left,
                        &right,
                    )
                    .returning_on_pane_close(return_buffer),
                );
            }
        }
    }

    pub(super) fn forget_revision_comparison(&mut self, buffer: usize) {
        self.git_state.comparisons.documents.remove(&buffer);
    }

    pub(super) fn resize_revision_comparisons(&mut self) {
        if self.git_state.comparisons.documents.is_empty() {
            return;
        }
        // Shared text uses the narrowest visible pane, without alternating layouts.
        let mut widths = std::collections::BTreeMap::<usize, usize>::new();
        for (pane, area) in &self.areas {
            let pane = &self.panes[pane];
            if pane.terminal.is_some()
                || !self
                    .git_state
                    .comparisons
                    .documents
                    .contains_key(&pane.buffer)
            {
                continue;
            }
            let width = usize::from(area.width.saturating_sub(8));
            widths
                .entry(pane.buffer)
                .and_modify(|old| *old = (*old).min(width))
                .or_insert(width);
        }
        for (buffer, width) in widths {
            self.resize_revision_comparison(buffer, width);
        }
    }

    fn resize_revision_comparison(&mut self, buffer: usize, width: usize) {
        let Some(document) = self.git_state.comparisons.documents.get_mut(&buffer) else {
            return;
        };
        if document.width == width {
            return;
        }
        document.width = width;
        let text = document.comparison.render(width);
        self.reproject_revision_list(buffer, &text, None);
    }
}

fn display_revision_path(path: &Option<PathBuf>) -> String {
    path.as_deref()
        .map(crate::git::display_path)
        .unwrap_or_else(|| "absent".into())
}
