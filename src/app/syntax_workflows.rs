// SPDX-License-Identifier: MPL-2.0

//! Syntax parse lifecycle, fold reconciliation, and presentation spans.

// Application-module dependencies:
use super::{
    App, HashSet, LanguageId, Mode, Offset, Range, ResolvedFold, Scope, Span, StaleSyntax,
    SyntaxEvent, SyntaxHandle, Transaction, buffer_language, parse_buffer,
};

impl App {
    /// Reparses inline until the production background worker is attached.
    pub(super) fn reparse(
        &mut self,
        buffer_id: usize,
        before: Option<&crate::text::Text>,
        transaction: &Transaction,
    ) {
        let Some(before) = before else {
            return;
        };
        let after = self.buffers[buffer_id].text().clone();
        if let Some(stale) = self.stale_syntax.get_mut(&buffer_id) {
            stale.append(before, &after, transaction);
            if let Some(worker) = &self.syntax_worker {
                worker.send(stale.request(buffer_id));
            }
            return;
        }
        let Some(syntax) = self.syntax[buffer_id].as_mut() else {
            return;
        };
        if let Some(worker) = &self.syntax_worker {
            let stale = StaleSyntax::new(syntax.clone(), before, &after, transaction);
            worker.send(stale.request(buffer_id));
            self.syntax[buffer_id] = None;
            self.stale_syntax.insert(buffer_id, stale);
        } else if !syntax.update(before, &after, transaction, &self.registry) {
            self.syntax[buffer_id] = None;
        }
    }

    /// Enables background reparsing. Tests remain inline unless they opt in.
    pub fn attach_syntax_worker(&mut self, worker: SyntaxHandle) {
        self.syntax_worker = Some(worker);
    }

    pub fn has_pending_syntax(&self) -> bool {
        !self.stale_syntax.is_empty()
    }

    /// Applies a finished parse between frames, rejecting any result whose
    /// syntax base or target text revision is no longer current.
    pub fn apply_syntax_event(&mut self, event: SyntaxEvent) -> bool {
        let buffer_id = event.buffer;
        let Some(stale) = self.stale_syntax.get(&buffer_id) else {
            return false;
        };
        if self
            .buffers
            .get(buffer_id)
            .is_none_or(|buffer| buffer.text().revision() != event.text_revision)
            || !stale.accepts(&event)
        {
            return false;
        }
        self.stale_syntax.remove(&buffer_id);
        self.syntax[buffer_id] = event.syntax;
        true
    }

    /// Redetects the language and reparses from scratch.
    ///
    /// Undo, redo, reload, and save-as can replace the first-line identity or
    /// path without producing a transaction the parser can consume. Going
    /// through the document inference boundary here prevents an old grammar
    /// from parsing text that now belongs to another language.
    pub(super) fn reparse_whole(&mut self, buffer_id: usize) {
        self.clear_syntax_history(buffer_id);
        self.stale_syntax.remove(&buffer_id);
        self.syntax[buffer_id] = parse_buffer(&self.buffers[buffer_id], &self.registry);
    }

    /// Rebuilds every language-derived service after text was replaced without
    /// a usable transaction, preserving full-document LSP sync only when the
    /// inferred language identity stayed stable.
    pub(super) fn resync_replaced_buffer(
        &mut self,
        buffer_id: usize,
        language_before: Option<LanguageId>,
    ) {
        self.invalidate_partial_guards(buffer_id);
        if !self.buffers[buffer_id].is_read_only() {
            self.word_index_notify_update(buffer_id);
        }
        let language_after = buffer_language(&self.buffers[buffer_id], &self.registry);
        self.reparse_whole(buffer_id);
        if language_before == language_after {
            self.lsp_resync(buffer_id);
        } else {
            self.retire_lsp_buffer(buffer_id);
            self.lsp_touch(buffer_id);
        }
    }

    pub(super) fn clear_syntax_history(&mut self, buffer_id: usize) {
        for pane in self
            .panes
            .values_mut()
            .filter(|pane| pane.buffer == buffer_id)
        {
            pane.syntax_history.clear();
            pane.folds.clear();
        }
    }

    pub(super) fn resolved_folds(&self, pane_id: usize) -> Vec<ResolvedFold> {
        let pane = &self.panes[&pane_id];
        let buffer = &self.buffers[pane.buffer];
        let Some(syntax) = self.syntax[pane.buffer].as_ref() else {
            return Vec::new();
        };
        let mut resolved = pane
            .folds
            .collapsed
            .iter()
            .filter_map(|source| {
                let range = syntax.resolve_fold_range(buffer.text(), *source).ok()?;
                let from = buffer.position_of(range.from);
                let to = buffer.position_of(range.to);
                let first_hidden_row = from.row.saturating_add(1);
                let end_hidden_row = to.row.saturating_add(usize::from(to.col > 0));
                (first_hidden_row < end_hidden_row).then_some(ResolvedFold {
                    source: *source,
                    anchor_row: from.row,
                    first_hidden_row,
                    end_hidden_row,
                })
            })
            .collect::<Vec<_>>();
        resolved.sort_by_key(|fold| (fold.anchor_row, fold.end_hidden_row));
        resolved
    }

    pub(super) fn reveal_active_selection_from_folds(&mut self) {
        self.reveal_pane_selection_from_folds(self.active_pane);
    }

    pub(super) fn reveal_pane_selection_from_folds(&mut self, pane_id: usize) {
        let buffer_id = self.panes[&pane_id].buffer;
        let rows = self.panes[&pane_id]
            .selection
            .ranges()
            .iter()
            .map(|range| self.buffers[buffer_id].offset_to_row(range.head))
            .collect::<Vec<_>>();
        let resolved = self.resolved_folds(pane_id);
        let hidden = resolved
            .iter()
            .filter(|fold| rows.iter().any(|row| fold.hides(*row)))
            .map(|fold| fold.source)
            .collect::<HashSet<_>>();
        self.panes
            .get_mut(&pane_id)
            .unwrap()
            .folds
            .collapsed
            .retain(|fold| !hidden.contains(fold));
    }

    /// Highlight spans for a character range of a buffer, empty when the
    /// buffer has no syntax tree.
    pub fn highlights(&self, buffer_id: usize, from: Offset, to: Offset) -> Vec<Span> {
        if self.buffers[buffer_id].is_git_branches() {
            return self.git_branch_highlights(buffer_id, from, to);
        }
        if let Some(syntax) = self.syntax.get(buffer_id).and_then(Option::as_ref) {
            return syntax.spans(self.buffers[buffer_id].text(), &self.registry, from, to);
        }
        self.stale_syntax
            .get(&buffer_id)
            .map(|syntax| {
                syntax
                    .translated_spans(self.buffers[buffer_id].text(), &self.registry, from, to)
                    .into_spans()
            })
            .unwrap_or_default()
    }

    /// The upstream and worktree annotations in the branch list, as spans.
    ///
    /// The list is projected rather than parsed, so its spans come from the
    /// projection instead of from a grammar. They carry the `comment` scope
    /// because that is what the annotation is — a note beside the name, dimmed
    /// by every theme — and because borrowing an existing scope keeps this out
    /// of the business of choosing colours.
    /// Where one row of the changed-file list keeps its two counts.
    ///
    /// Not a highlight span like the branch list's annotations: those borrow a
    /// syntax scope because a note beside a name is whatever the theme dims,
    /// while these two numbers are an addition and a removal and belong in the
    /// colours the gutter and every diff already give those.
    pub(crate) fn git_status_count_columns(
        &self,
        buffer_id: usize,
        row: usize,
    ) -> Option<&crate::git::CountColumns> {
        self.buffers
            .get(buffer_id)
            .filter(|buffer| buffer.is_git_status())?;
        self.git_state.status_counts().get(row)?.as_ref()
    }

    fn git_branch_highlights(&self, buffer_id: usize, from: Offset, to: Offset) -> Vec<Span> {
        let Some(scope) = Scope::named("comment") else {
            return Vec::new();
        };
        let buffer = &self.buffers[buffer_id];
        let first = buffer.offset_to_row(from);
        let last = buffer.offset_to_row(to);
        (first..=last)
            .filter_map(|row| {
                let (start, end) = self.git_state.branch_rows().get(row)?.annotation?;
                let line = buffer.line_to_offset(row);
                let span = Span {
                    from: (line + start).max(from),
                    to: (line + end).min(to),
                    scope,
                };
                (span.from < span.to).then_some(span)
            })
            .collect()
    }

    pub(crate) fn normalize_buffer(&mut self, buffer_id: usize) {
        let buffer = &self.buffers[buffer_id];
        let insert = self.mode == Mode::Insert;
        for pane in self
            .panes
            .values_mut()
            .filter(|pane| pane.buffer == buffer_id)
        {
            let selection = pane.selection.transform(|range| {
                let head = buffer.clamp_offset(range.head, insert);
                if range.is_empty() {
                    // Clamping the two ends under different rules would turn a
                    // caret at end-of-line into a one-character selection, and
                    // the next keystroke would replace that character.
                    Range::point(head)
                } else {
                    Range::new(buffer.clamp_offset(range.anchor, false), head)
                }
            });
            pane.replace_selection(selection);
            pane.preserve_scroll = false;
        }
    }
}
