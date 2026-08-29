// SPDX-License-Identifier: MPL-2.0

//! Selection-first movement, transactional edits, registers, and syntax actions.

// Application-module dependencies:
use super::{
    App, BTreeMap, Buffer, Change, DelimiterPair, DirectoryRegister, HashSet, HistoryReset, Jump,
    JumpLabels, KeyCode, KeyStroke, LanguageId, ListAction, ListPicker, Mode, Modifiers, Motion,
    Offset, Outline, Pane, PickerItem, Press, Range, Regex, Register, Result,
    SearchSelectionPresentation, Selection, SelectionSemantics, ShrinkResult, SyntaxError,
    SyntaxObject, SyntaxObjectPart, SyntaxSelectionRange, SyntaxSelectionTransform, TerminalId,
    Transaction, TransferMode, buffer_language, column_at_visual_column, fold_degradation_suffix,
    insert_word_back, insert_word_forward, is_single_cell, is_word, merged_line_spans, move_offset,
    move_offset_projected, navigate_text_object, operative_span, outline_item_detail,
    outline_status, parse_buffer, project_visible_rows, select_delimiter, select_text_object,
    syntax_object_label, syntax_object_part_label, trailing_whitespace_changes,
    transform_selection, visual_column, without_trailing_line_terminator,
};

impl App {
    pub(super) fn active_mut(&mut self) -> &mut Pane {
        self.panes.get_mut(&self.active_pane).unwrap()
    }

    /// Records the active pane's position before a non-local move.
    ///
    /// Called by the handful of operations that take the caret somewhere it
    /// could not have walked to: opening a buffer, following a language-server
    /// result, starting a search, and jumping to a file boundary.
    pub(super) fn push_jump(&mut self) {
        let pane_id = self.active_pane;
        let pane = &self.panes[&pane_id];
        let jump = Jump::new(
            pane.buffer,
            pane.selection.clone(),
            pane.selection_semantics(),
        )
        .with_terminal(pane.terminal.map(TerminalId::get));
        self.panes.get_mut(&pane_id).unwrap().jumps.push(jump);
    }

    pub(super) fn jump(&mut self, backward: bool) {
        self.jump_in(backward, false)
    }

    /// Steps through jump history, optionally only between buffers.
    ///
    /// `across_buffers` exists because the two traversals answer different
    /// questions: one retraces reading, the other retraces which file was open.
    /// They share one history, so a file-level step lands on a real recorded
    /// position rather than the top of the file.
    pub(super) fn jump_in(&mut self, backward: bool, across_buffers: bool) {
        let pane_id = self.active_pane;
        let here = {
            let pane = &self.panes[&pane_id];
            Jump::new(
                pane.buffer,
                pane.selection.clone(),
                pane.selection_semantics(),
            )
            .with_terminal(pane.terminal.map(TerminalId::get))
        };
        let pane = self.panes.get_mut(&pane_id).unwrap();
        let target = match (backward, across_buffers) {
            (true, false) => pane.jumps.backward(here),
            (true, true) => pane.jumps.backward_across_buffers(here),
            (false, false) => pane.jumps.forward(),
            (false, true) => pane.jumps.forward_across_buffers(&here),
        };
        let Some(target) = target else {
            self.status(match (backward, across_buffers) {
                (true, false) => "no earlier position",
                (true, true) => "no earlier buffer",
                (false, false) => "no later position",
                (false, true) => "no later buffer",
            });
            return;
        };
        // Remembered selections are mapped through every transaction, but a
        // buffer can also be replaced wholesale, so clamp before trusting them.
        let Some(buffer) = self.buffers.get(target.buffer) else {
            self.action_failed("that position's buffer is gone");
            return;
        };
        let selection = target.selection.transform(|range| {
            Range::new(
                buffer.clamp_offset(range.anchor, false),
                buffer.clamp_offset(range.head, false),
            )
        });
        let terminal = target
            .terminal
            .map(TerminalId::from_raw)
            .filter(|id| self.terminals.get(*id).is_some());
        {
            let pane = self.panes.get_mut(&pane_id).unwrap();
            pane.retarget(target.buffer);
            pane.replace_selection(selection);
            pane.mark_selection_semantics(target.semantics);
            pane.preserve_scroll = false;
        }
        if let Some(id) = terminal {
            self.move_terminal_to_pane(id, pane_id);
            self.last_terminal = Some(id);
        }
        self.mode = Mode::Normal;
        self.status(if backward {
            "jumped back"
        } else {
            "jumped forward"
        });
    }

    pub(super) fn motion(&mut self, motion: Motion) {
        self.motion_with_extension(motion, self.mode == Mode::Select);
    }

    pub(super) fn motion_with_extension(&mut self, motion: Motion, extend: bool) {
        let pane_id = self.active_pane;
        self.panes.get_mut(&pane_id).unwrap().preserve_scroll = false;
        let buffer_id = self.panes[&pane_id].buffer;
        let viewport_height = self.viewport_height();
        let scroll_row = self.panes[&pane_id].scroll_row;
        let scroll_wrap = self.panes[&pane_id].scroll_wrap;
        let wrap_width = self.panes[&pane_id].wrap_width.max(1);
        let soft_wrap = self.pane_soft_wrap(pane_id);
        let tab_width = self.config.editor.tab_width;
        let buffer = &self.buffers[buffer_id];
        let folds = self.resolved_folds(pane_id);
        let diff = self.diff_projection(pane_id);
        let selection = self.panes[&pane_id].selection.transform(|range| {
            let head = if matches!(
                motion,
                Motion::Up
                    | Motion::Down
                    | Motion::PageUp
                    | Motion::PageDown
                    | Motion::HalfPageUp
                    | Motion::HalfPageDown
                    | Motion::WindowTop
                    | Motion::WindowCenter
                    | Motion::WindowBottom
            ) {
                move_offset_projected(
                    buffer,
                    range.head,
                    motion,
                    (viewport_height, scroll_row, scroll_wrap),
                    wrap_width,
                    tab_width,
                    soft_wrap,
                    &folds,
                    diff,
                )
            } else {
                move_offset(buffer, range.head, motion, viewport_height, scroll_row)
            };
            if extend {
                range.extend_to(head)
            } else {
                Range::point(head)
            }
        });
        self.panes
            .get_mut(&pane_id)
            .unwrap()
            .replace_selection(selection);
        self.reveal_active_selection_from_folds();
    }

    pub(super) fn goto_line(&mut self, one_based: usize) {
        let buffer = self.active_buffer();
        let row = one_based.saturating_sub(1).min(buffer.last_row());
        let head = buffer.clamp_offset(buffer.line_to_offset(row), false);
        let selection = if self.mode == Mode::Select {
            self.active()
                .selection
                .transform(|range| range.extend_to(head))
        } else {
            Selection::point(head)
        };
        self.push_jump();
        self.active_mut().replace_selection(selection);
    }

    /// Paints a jump label over every eligible word visible in the active pane.
    ///
    /// A word remains eligible when its first two characters are single-cell,
    /// but a one-key label needs only its first cell to be visible. Candidates
    /// are ranked in projected screen space, so folds and wraps cannot make a
    /// distant document offset look close. Label assignment removes and
    /// regenerates any two-key label that would cross the viewport edge.
    pub(super) fn label_visible_words(&mut self) {
        let height = self.viewport_height();
        let tab_width = self.config.editor.tab_width;
        let soft_wrap = self.pane_soft_wrap(self.active_pane);
        let pane = self.active();
        // The width the pane actually wraps at, gutter already subtracted, as
        // recorded by the last frame. Recomputing it here would disagree with
        // what is on screen by the width of the line-number column.
        let (first_row, scroll_wrap, scroll_col, wrap_width) = (
            pane.scroll_row,
            pane.scroll_wrap,
            pane.scroll_col,
            pane.wrap_width.max(1),
        );
        let buffer = self.active_buffer();
        let cursor = pane.cursor(buffer);
        let folds = self.resolved_folds(self.active_pane);

        // A wrapped row occupies several screen rows, so the last text row on
        // screen is not `scroll_row + height`, and a row can be on screen with
        // only some of its columns drawn. Both come from the wrapper, which is
        // the same geometry rendering uses.
        let projected = project_visible_rows(
            buffer,
            &folds,
            first_row,
            scroll_wrap,
            height,
            wrap_width,
            tab_width,
            soft_wrap,
            self.diff_projection(self.active_pane),
        );
        let cursor_screen_row = projected
            .iter()
            .position(|visual| {
                visual.document_row == Some(cursor.row)
                    && visual.segment.is_none_or(|segment| {
                        cursor.col >= segment.start
                            && (cursor.col < segment.end
                                || cursor.col == segment.end
                                    && segment.end == buffer.line_len(cursor.row))
                    })
            })
            .unwrap_or_else(|| {
                let first = projected.iter().find_map(|row| row.document_row);
                if first.is_some_and(|row| cursor.row < row) {
                    0
                } else {
                    projected.len().saturating_sub(1)
                }
            });
        let cursor_line = buffer.line_string(cursor.row);
        let cursor_screen_col = projected
            .get(cursor_screen_row)
            .filter(|visual| visual.document_row == Some(cursor.row))
            .map_or(0, |visual| {
                visual.segment.map_or_else(
                    || {
                        crate::wrap::cells_from_column(
                            &cursor_line,
                            scroll_col,
                            cursor.col,
                            tab_width,
                        )
                    },
                    |segment| {
                        crate::wrap::display_column(&cursor_line, cursor.col, tab_width)
                            .saturating_sub(segment.start_cell)
                    },
                )
            })
            .min(wrap_width.saturating_sub(1));

        #[derive(Clone, Copy)]
        struct Candidate {
            offset: Offset,
            screen_row: usize,
            screen_col: usize,
            visible_label_cells: usize,
            order: usize,
        }

        let mut candidates = Vec::new();
        for (screen_row, visual) in projected.into_iter().enumerate() {
            // Filler and padding carry no text, so they offer nothing to label.
            let Some(row) = visual.document_row else {
                continue;
            };
            let columns = visual
                .segment
                .map_or(scroll_col..usize::MAX, |segment| segment.start..segment.end);
            let start = buffer.line_to_offset(row);
            let line_string = buffer.line_string(row);
            let line: Vec<char> = line_string.chars().collect();
            let mut word_start = None;
            // One past the end closes a word that runs to the end of the row.
            for column in 0..=line.len() {
                match (line.get(column).copied().is_some_and(is_word), word_start) {
                    (true, None) => word_start = Some(column),
                    (false, Some(begin)) => {
                        if column - begin >= 2
                            && columns.contains(&begin)
                            && line[begin..begin + 2].iter().copied().all(is_single_cell)
                        {
                            let screen_col = visual.segment.map_or_else(
                                || {
                                    crate::wrap::cells_from_column(
                                        &line_string,
                                        scroll_col,
                                        begin,
                                        tab_width,
                                    )
                                },
                                |segment| {
                                    crate::wrap::display_column(&line_string, begin, tab_width)
                                        .saturating_sub(segment.start_cell)
                                },
                            );
                            let visible_label_cells = visual.segment.map_or_else(
                                || wrap_width.saturating_sub(screen_col).min(2),
                                |segment| (segment.end - begin).min(2),
                            );
                            if visible_label_cells > 0 && screen_col < wrap_width {
                                candidates.push(Candidate {
                                    offset: start + begin,
                                    screen_row,
                                    screen_col,
                                    visible_label_cells,
                                    order: candidates.len(),
                                });
                            }
                        }
                        word_start = None;
                    }
                    _ => {}
                }
            }
        }

        candidates.sort_by_key(|candidate| {
            (
                candidate.screen_row.abs_diff(cursor_screen_row) * 10
                    + candidate.screen_col.abs_diff(cursor_screen_col),
                candidate.order,
            )
        });

        match JumpLabels::with_visible_lengths(
            candidates
                .into_iter()
                .map(|candidate| (candidate.offset, candidate.visible_label_cells)),
        ) {
            Some(labels) => {
                self.status(format!("jump to word: {} labels", labels.len()));
                self.jump = Some(labels);
            }
            None => self.action_failed("no words on screen to jump to"),
        }
    }

    pub(super) fn handle_jump_labels(&mut self, key: KeyStroke) -> Result<()> {
        // Only a plain character can name a label, so everything else —
        // Escape, a chord, an arrow — is a way out.
        let typed = match key.code {
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(Modifiers::CONTROL | Modifiers::ALT) =>
            {
                character
            }
            _ => {
                self.jump = None;
                self.status("jump cancelled");
                return Ok(());
            }
        };

        let mut labels = self.jump.take().expect("jump labels are live");
        match labels.press(typed) {
            Press::Narrowed => {
                self.jump = Some(labels);
                self.status("jump to word: second label key");
            }
            Press::Jumped(offset) => {
                if let Some(id) = self.active_terminal() {
                    let (_, rows) = self.pane_cells(self.active_pane);
                    let scroll_offset = self.config.editor.scroll_offset;
                    if let Some(session) = self.terminals.get_mut(id) {
                        session.goto_review_offset(offset, self.mode == Mode::Select);
                        session.focus_review_selection(rows.max(1), scroll_offset);
                    }
                    self.terminals.enforce_memory_budget();
                } else {
                    self.jump_to_word(offset);
                }
                self.status("");
            }
            Press::Missed => self.action_failed(format!("no jump label matches '{typed}'")),
        }
        Ok(())
    }

    /// Moves to a labelled word, extending the selection in Select mode as any
    /// other motion would.
    fn jump_to_word(&mut self, offset: Offset) {
        let head = self.active_buffer().clamp_offset(offset, false);
        let selection = if self.mode == Mode::Select {
            self.active()
                .selection
                .transform(|range| range.extend_to(head))
        } else {
            Selection::point(head)
        };
        self.push_jump();
        self.active_mut().replace_selection(selection);
    }

    /// Applies a transaction to the active buffer and maps every pane's
    /// selection through it, so panes sharing a buffer stay consistent.
    pub(crate) fn edit(&mut self, transaction: Transaction) -> bool {
        let buffer_id = self.active().buffer;
        if matches!(self.mode, Mode::Insert | Mode::Replace) {
            self.buffers[buffer_id].begin_undo_group();
        }
        let changed = self.apply_to_buffer(buffer_id, &transaction);
        if changed {
            self.report_new_registry_errors();
        }
        changed
    }

    /// The single path from a transaction to a changed buffer.
    ///
    /// Language-server edits reach buffers other than the active one, so this
    /// takes the buffer explicitly; everything that keeps a derived view of the
    /// text consistent — the syntax tree, the servers, and every pane's
    /// selection — happens here and nowhere else.
    pub(crate) fn apply_to_buffer(&mut self, buffer_id: usize, transaction: &Transaction) -> bool {
        let language_before = buffer_language(&self.buffers[buffer_id], &self.registry);
        // Both the syntax tree and `didChange` need the pre-edit text: one to
        // convert character offsets into tree-sitter's bytes, the other into
        // the server's own column encoding. Snapshot it once, and only when
        // something is actually watching.
        let watched = self.syntax[buffer_id].is_some()
            || self.stale_syntax.contains_key(&buffer_id)
            || self.lsp_documents.contains_key(&buffer_id);
        let before = watched.then(|| self.buffers[buffer_id].text().clone());
        if !self.buffers[buffer_id].apply(transaction) {
            return false;
        }
        if self.mode == Mode::Replace && self.active().buffer == buffer_id {
            // Replace's own overwrites and restorations take the session out
            // around this call. Every other mutation of the active Replace
            // buffer invalidates inverse ranges recorded against older text.
            self.replace_session = None;
        }
        self.reconcile_applied_transaction(
            buffer_id,
            language_before,
            before.as_ref(),
            transaction,
        );
        true
    }

    pub(super) fn reconcile_applied_transaction(
        &mut self,
        buffer_id: usize,
        language_before: Option<LanguageId>,
        before: Option<&crate::text::Text>,
        transaction: &Transaction,
    ) -> bool {
        self.invalidate_partial_guards(buffer_id);
        if !self.buffers[buffer_id].is_read_only() {
            self.word_index_notify_update(buffer_id);
        }
        let language_after = buffer_language(&self.buffers[buffer_id], &self.registry);
        self.clear_syntax_history(buffer_id);
        let synchronized = if language_before == language_after {
            self.reparse(buffer_id, before, transaction);
            if let Some(before) = before {
                self.lsp_change(buffer_id, before, transaction)
            } else {
                true
            }
        } else {
            self.stale_syntax.remove(&buffer_id);
            self.syntax[buffer_id] = parse_buffer(&self.buffers[buffer_id], &self.registry);
            self.retire_lsp_buffer(buffer_id);
            self.lsp_touch(buffer_id);
            true
        };
        self.map_transaction_views(buffer_id, std::slice::from_ref(transaction));
        synchronized
    }

    /// The half-open span a user-facing operation acts on. This includes the
    /// character under the caret, because Normal and Select modes draw a block
    /// caret sitting *on* a character rather than between two.
    pub(super) fn operative_spans(&self) -> Vec<(Offset, Offset)> {
        if matches!(
            self.active().selection_semantics(),
            SelectionSemantics::HalfOpen | SelectionSemantics::VimLinewise
        ) {
            return self
                .active()
                .selection
                .ranges()
                .iter()
                .map(|range| (range.from(), range.to()))
                .collect();
        }
        let buffer = self.active_buffer();
        self.active()
            .selection
            .ranges()
            .iter()
            .map(|range| operative_span(buffer, range))
            .collect()
    }

    pub(super) fn insert_char(&mut self, ch: char) {
        let selection = self.active().selection.clone();
        let transaction = selection.change_by(|_| Some(ch.to_string()));
        self.edit(transaction);
    }

    pub(super) fn insert_text(&mut self, text: &str) {
        let selection = self.active().selection.clone();
        let transaction = selection.change_by(|_| Some(text.to_owned()));
        self.edit(transaction);
    }

    pub(super) fn enter_replace_mode(&mut self) {
        let buffer_id = self.active().buffer;
        let selection = self
            .active()
            .selection
            .transform(|range| Range::point(range.head));
        self.active_mut().replace_selection(selection);
        self.buffers[buffer_id].begin_undo_group();
        self.replace_session = Some(super::ReplaceSession {
            buffer: buffer_id,
            steps: Vec::new(),
        });
        self.mode = Mode::Replace;
    }

    /// Overwrites one character at every Replace caret, appending at line end.
    /// Line terminators are structural: a typed newline inserts one, while no
    /// other character is allowed to consume LF or either half of CRLF.
    pub(super) fn replace_mode_text(&mut self, text: &str) {
        let mut characters = text.chars().peekable();
        while let Some(character) = characters.next() {
            if character == '\r' && characters.peek() == Some(&'\n') {
                characters.next();
                self.replace_mode_character('\n');
            } else {
                self.replace_mode_character(character);
            }
        }
    }

    fn replace_mode_character(&mut self, character: char) {
        let buffer_id = self.active().buffer;
        let before = self.active().selection.clone();
        let buffer = self.active_buffer();
        let changes = before
            .ranges()
            .iter()
            .map(|range| {
                let head = range.head;
                if character == '\n' {
                    let row = buffer.offset_to_row(head);
                    Change::new(head, head, preferred_line_ending(buffer, row))
                } else {
                    let row = buffer.offset_to_row(head);
                    let row_end = buffer.line_to_offset(row) + buffer.line_len(row);
                    if head < row_end {
                        Change::new(head, head + 1, character.to_string())
                    } else {
                        Change::new(head, head, character.to_string())
                    }
                }
            })
            .collect::<Vec<_>>();
        let transaction = Transaction::new(changes);
        let mut preview = self.buffers[buffer_id].text().clone();
        let inverse = preview.apply(&transaction).into_transaction();
        let after = Selection::new(
            before
                .ranges()
                .iter()
                .map(|range| {
                    let start = transaction.map_offset(range.head, crate::text::Assoc::Before);
                    let inserted = if character == '\n' {
                        let row = self.active_buffer().offset_to_row(range.head);
                        preferred_line_ending(self.active_buffer(), row)
                            .chars()
                            .count()
                    } else {
                        1
                    };
                    Range::point(start + inserted)
                })
                .collect(),
            before.primary_index(),
        );
        let mut session = self
            .replace_session
            .take()
            .unwrap_or(super::ReplaceSession {
                buffer: buffer_id,
                steps: Vec::new(),
            });
        if session.buffer != buffer_id {
            session = super::ReplaceSession {
                buffer: buffer_id,
                steps: Vec::new(),
            };
        }
        if self.edit(transaction) {
            self.active_mut().replace_selection(after.clone());
            session.steps.push(super::ReplaceStep {
                before,
                after,
                inverse,
            });
        }
        self.replace_session = Some(session);
    }

    pub(super) fn restore_replace_step(&mut self) -> bool {
        let buffer_id = self.active().buffer;
        let current = self.active().selection.clone();
        let Some(mut session) = self.replace_session.take() else {
            return false;
        };
        if session.buffer != buffer_id
            || session
                .steps
                .last()
                .is_none_or(|step| step.after != current)
        {
            self.replace_session = Some(session);
            return false;
        }
        let step = session.steps.pop().unwrap();
        let changed = self.edit(step.inverse);
        if changed {
            self.active_mut().replace_selection(step.before);
        }
        self.replace_session = Some(session);
        changed
    }

    pub(super) fn restore_replace_word(&mut self) {
        let target = insert_word_back(self.active_buffer(), self.active().selection.primary().head);
        while self.active().selection.primary().head > target && self.restore_replace_step() {}
    }

    pub(super) fn restore_replace_line(&mut self) {
        let head = self.active().selection.primary().head;
        let row = self.active_buffer().offset_to_row(head);
        let target = self.active_buffer().line_to_offset(row);
        while self.active().selection.primary().head > target && self.restore_replace_step() {}
    }

    pub(super) fn edit_newline(&mut self) {
        fn canonical_roman(marker: &str) -> bool {
            fn take_prefix(input: &mut &str, prefixes: &[&str]) {
                if let Some(prefix) = prefixes.iter().find(|prefix| input.starts_with(**prefix)) {
                    *input = &input[prefix.len()..];
                }
            }

            if marker.is_empty()
                || !marker
                    .chars()
                    .all(|character| matches!(character, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M'))
            {
                return false;
            }
            let mut rest = marker;
            take_prefix(&mut rest, &["MMM", "MM", "M"]);
            take_prefix(
                &mut rest,
                &["CM", "CD", "DCCC", "DCC", "DC", "D", "CCC", "CC", "C"],
            );
            take_prefix(
                &mut rest,
                &["XC", "XL", "LXXX", "LXX", "LX", "L", "XXX", "XX", "X"],
            );
            take_prefix(
                &mut rest,
                &["IX", "IV", "VIII", "VII", "VI", "V", "III", "II", "I"],
            );
            rest.is_empty()
        }

        fn list_continuation_indent(before_caret: &str) -> Option<String> {
            let indent_end = before_caret
                .char_indices()
                .find_map(|(index, character)| (!matches!(character, ' ' | '\t')).then_some(index))
                .unwrap_or(before_caret.len());
            let body = &before_caret[indent_end..];
            let marker_end = match body.chars().next()? {
                '-' | '*' | '+' => 1,
                _ => {
                    let period = body.find('.')?;
                    let marker = &body[..period];
                    let characters = marker.chars().count();
                    let decimal = !marker.is_empty()
                        && marker.chars().all(|character| character.is_ascii_digit());
                    let letter = characters == 1
                        && marker
                            .chars()
                            .all(|character| character.is_ascii_alphabetic());
                    let roman = characters > 1 && canonical_roman(marker);
                    if !(decimal || letter || roman) {
                        return None;
                    }
                    period + 1
                }
            };
            let after_marker = &body[marker_end..];
            let separator_end = after_marker
                .char_indices()
                .find_map(|(index, character)| (!matches!(character, ' ' | '\t')).then_some(index))
                .unwrap_or(after_marker.len());
            if separator_end == 0 {
                return None;
            }

            Some(format!(
                "{}{}{}",
                &before_caret[..indent_end],
                " ".repeat(body[..marker_end].chars().count()),
                &after_marker[..separator_end]
            ))
        }

        let buffer_id = self.active().buffer;
        let selection = self.active().selection.clone();
        let buffer = &self.buffers[buffer_id];
        let syntax = self.syntax[buffer_id].as_ref();
        let unit = " ".repeat(self.config.editor.tab_width.max(1));
        let smart_newline = self.config.editor.smart_newline;
        // Every answer is derived from the same pre-edit text. This matters
        // for multi-caret insertion: an earlier caret must never change the
        // syntax or leading whitespace observed by a later one.
        let replacements = selection
            .ranges()
            .iter()
            .map(|range| {
                // `Selection::change_by` replaces at the normalized start of
                // the range, so indentation must be derived from that same
                // insertion point regardless of selection direction.
                let row = buffer.offset_to_row(range.from());
                let line_start = buffer.line_to_offset(row);
                let before_caret = buffer.slice(line_start, range.from());
                let prefix = before_caret
                    .chars()
                    .take_while(|character| matches!(character, ' ' | '\t'))
                    .collect::<String>();
                let existing_terminator = line_terminator(buffer, row);
                let terminator = existing_terminator
                    .map(|(terminator, _)| terminator)
                    .unwrap_or_else(|| preferred_line_ending(buffer, row));
                if !smart_newline {
                    return format!("{terminator}{prefix}");
                }
                let list_indent = list_continuation_indent(&before_caret);
                // The syntax contract answers for an existing newline token,
                // so a mid-line caret deliberately probes its pre-edit row
                // terminator. An unterminated final row has no truthful token
                // to query and falls back to its exact leading prefix.
                let newline_offset = existing_terminator.map(|(_, offset)| offset);
                let syntax_unit = list_indent.is_none().then(|| {
                    syntax
                        .zip(newline_offset)
                        .and_then(|(syntax, newline_offset)| {
                            syntax
                                .newline_indent(buffer.text(), &self.registry, newline_offset)
                                .ok()
                        })
                        .map_or("", |indent| {
                            if indent.tab_levels > 0 {
                                "\t"
                            } else if indent.begin_levels + indent.always_levels > 0 {
                                unit.as_str()
                            } else {
                                ""
                            }
                        })
                });
                format!(
                    "{terminator}{}{}",
                    list_indent.as_deref().unwrap_or(&prefix),
                    syntax_unit.unwrap_or("")
                )
            })
            .collect::<Vec<_>>();
        let mut index = 0;
        let transaction = selection.change_by(|_| {
            let replacement = replacements[index].clone();
            index += 1;
            Some(replacement)
        });
        self.edit(transaction);
    }

    pub(super) fn edit_backspace(&mut self) {
        let buffer_id = self.active().buffer;
        let spans = self
            .active()
            .selection
            .ranges()
            .iter()
            .filter_map(|range| {
                if !range.is_empty() {
                    return Some((range.from(), range.to()));
                }
                (range.head > 0).then_some((range.head - 1, range.head))
            })
            .collect::<Vec<_>>();
        let changes = crlf_safe_deletions(&self.buffers[buffer_id], spans);
        self.edit(Transaction::new(changes));
        self.normalize_buffer(buffer_id);
    }

    pub(super) fn edit_delete(&mut self) {
        let buffer_id = self.active().buffer;
        let len = self.active_buffer().len_chars();
        let spans = self
            .active()
            .selection
            .ranges()
            .iter()
            .filter_map(|range| {
                if !range.is_empty() {
                    return Some((range.from(), range.to()));
                }
                (range.head < len).then_some((range.head, range.head + 1))
            })
            .collect::<Vec<_>>();
        let changes = crlf_safe_deletions(&self.buffers[buffer_id], spans);
        self.edit(Transaction::new(changes));
        self.normalize_buffer(buffer_id);
    }

    pub(super) fn delete_word_backward(&mut self) {
        let buffer_id = self.active().buffer;
        let buffer = &self.buffers[buffer_id];
        let changes = self
            .active()
            .selection
            .ranges()
            .iter()
            .filter_map(|range| {
                let start = insert_word_back(buffer, range.head);
                (start < range.head).then(|| Change::new(start, range.head, ""))
            })
            .collect();
        self.edit(Transaction::new(changes));
        self.normalize_buffer(buffer_id);
    }

    pub(super) fn delete_word_forward(&mut self) {
        let buffer_id = self.active().buffer;
        let buffer = &self.buffers[buffer_id];
        let changes = self
            .active()
            .selection
            .ranges()
            .iter()
            .filter_map(|range| {
                let end = insert_word_forward(buffer, range.head);
                (range.head < end).then(|| Change::new(range.head, end, ""))
            })
            .collect();
        self.edit(Transaction::new(changes));
        self.normalize_buffer(buffer_id);
    }

    pub(super) fn delete_to_line_start(&mut self) {
        let buffer_id = self.active().buffer;
        let buffer = &self.buffers[buffer_id];
        let changes = self
            .active()
            .selection
            .ranges()
            .iter()
            .filter_map(|range| {
                let start = buffer.line_to_offset(buffer.offset_to_row(range.head));
                (start < range.head).then(|| Change::new(start, range.head, ""))
            })
            .collect();
        self.edit(Transaction::new(changes));
        self.normalize_buffer(buffer_id);
    }

    pub(super) fn delete_to_line_end(&mut self) {
        let buffer_id = self.active().buffer;
        let buffer = &self.buffers[buffer_id];
        let changes = self
            .active()
            .selection
            .ranges()
            .iter()
            .filter_map(|range| {
                let row = buffer.offset_to_row(range.head);
                let end = buffer.line_to_offset(row) + buffer.line_len(row);
                (range.head < end).then(|| Change::new(range.head, end, ""))
            })
            .collect();
        self.edit(Transaction::new(changes));
        self.normalize_buffer(buffer_id);
    }

    pub(super) fn insert_indentation(&mut self) {
        let width = self.config.editor.tab_width.max(1);
        let buffer = self.active_buffer();
        let selection = self.active().selection.clone();
        let indents: Vec<String> = selection
            .ranges()
            .iter()
            .map(|range| {
                let position = buffer.position_of(range.head);
                let column = visual_column(&buffer.line_string(position.row), position.col, width);
                " ".repeat(width - column % width)
            })
            .collect();
        let mut index = 0;
        let transaction = selection.change_by(|_| {
            let text = indents[index].clone();
            index += 1;
            Some(text)
        });
        self.edit(transaction);
    }

    pub(super) fn open_line(&mut self, above: bool) {
        let buffer_id = self.active().buffer;
        self.buffers[buffer_id].begin_undo_group();
        let buffer = &self.buffers[buffer_id];
        let mut rows: Vec<usize> = self
            .active()
            .selection
            .ranges()
            .iter()
            .map(|range| buffer.offset_to_row(range.head))
            .collect();
        rows.sort_unstable();
        rows.dedup();

        let points: Vec<Offset> = rows
            .iter()
            .map(|row| {
                if above {
                    buffer.line_to_offset(*row)
                } else {
                    buffer.line_to_offset(*row) + buffer.line_len(*row)
                }
            })
            .collect();

        let insertions = points
            .iter()
            .zip(&rows)
            .map(|(point, row)| (*point, preferred_line_ending(buffer, *row)))
            .collect::<Vec<_>>();
        // Earlier insertions shift every later caret by their complete line
        // terminator. Opening below lands after its new terminator; opening
        // above lands before it, on the empty row just created.
        let mut inserted = 0;
        let heads: Vec<Offset> = insertions
            .iter()
            .map(|(point, terminator)| {
                let head = point + inserted + usize::from(!above) * terminator.len();
                inserted += terminator.len();
                head
            })
            .collect();

        let changes = insertions
            .into_iter()
            .map(|(point, terminator)| Change::new(point, point, terminator))
            .collect();
        if !self.edit(Transaction::new(changes)) {
            return;
        }
        let selection = Selection::new(heads.into_iter().map(Range::point).collect(), 0);
        self.active_mut().replace_selection(selection);
        self.mode = Mode::Insert;
    }

    /// Selects whole lines, then walks the moving edge one line at a time.
    ///
    /// The first press only snaps each range to the lines it already touches,
    /// which is what makes `x` on an empty line look like a no-op: there is no
    /// character to highlight yet. Every press after that moves the head row by
    /// one, down for `x` and up for `X`, so the two keys walk the same edge in
    /// opposite directions and `x x X` leaves exactly the line the walk began
    /// on. Row arithmetic drives this rather than the range's emptiness, so an
    /// empty line extends like any other.
    pub(super) fn select_line(&mut self, down: bool) {
        let buffer = self.active_buffer();
        let last_row = buffer.last_row();
        let live = self.line_select.is_some();
        let selection = self.active().selection.transform(|range| {
            let anchor_row = buffer.offset_to_row(range.anchor);
            let mut head_row = buffer.offset_to_row(range.head);
            if live {
                head_row = if down {
                    (head_row + 1).min(last_row)
                } else {
                    head_row.saturating_sub(1)
                };
            }
            // The anchor sits at the outer edge of its own row, so the span
            // always covers both rows in full whichever way it points.
            if head_row >= anchor_row {
                Range::new(
                    buffer.line_to_offset(anchor_row),
                    buffer.row_end_offset(head_row, false),
                )
            } else {
                Range::new(
                    buffer.row_end_offset(anchor_row, false),
                    buffer.line_to_offset(head_row),
                )
            }
        });
        self.active_mut().replace_selection(selection);
        if self.line_select.is_none() {
            self.line_select = Some(self.mode);
            self.mode = Mode::Select;
        }
    }

    pub(super) fn toggle_select_mode(&mut self) {
        if self.mode == Mode::Select {
            self.mode = Mode::Normal;
            let selection = self.active().selection.collapse();
            self.active_mut().replace_selection(selection);
        } else {
            self.mode = Mode::Select;
        }
    }

    pub(super) fn select_all(&mut self) {
        let buffer = self.active_buffer();
        let end = buffer.row_end_offset(buffer.last_row(), false);
        self.active_mut()
            .replace_selection(Selection::single(Range::new(0, end)));
        self.mode = Mode::Select;
    }

    pub(super) fn collapse_selection(&mut self) {
        let selection = self.active().selection.collapse();
        self.active_mut().replace_selection(selection);
        self.mode = Mode::Normal;
    }

    pub(super) fn flip_selection(&mut self) {
        let selection = self.active().selection.flip();
        self.active_mut().replace_selection(selection);
        self.mode = Mode::Select;
    }

    pub(super) fn expand_syntax_selection(&mut self) -> Result<()> {
        let buffer_id = self.active().buffer;
        let pane_id = self.active_pane;
        let selection = self.active().selection.clone();
        let mode = self.mode;
        let semantics = self.active().selection_semantics();
        let Some(syntax) = self.syntax[buffer_id].as_ref() else {
            self.mark_unavailable("syntax is unavailable for this buffer");
            return Ok(());
        };
        let expanded = self
            .panes
            .get_mut(&pane_id)
            .unwrap()
            .syntax_history
            .expand(
                syntax,
                self.buffers[buffer_id].text(),
                &self.registry,
                &selection,
                mode,
                semantics,
            )?;
        let Some(expanded) = expanded else {
            self.status("no larger syntax selection");
            return Ok(());
        };
        self.active_mut().replace_selection(expanded);
        self.active_mut()
            .mark_selection_semantics(SelectionSemantics::HalfOpen);
        self.mode = Mode::Select;
        Ok(())
    }

    pub(super) fn shrink_syntax_selection(&mut self) -> Result<()> {
        let buffer_id = self.active().buffer;
        let pane_id = self.active_pane;
        let selection = self.active().selection.clone();
        let mode = self.mode;
        let semantics = self.active().selection_semantics();
        let Some(syntax) = self.syntax[buffer_id].as_ref() else {
            self.mark_unavailable("syntax is unavailable for this buffer");
            return Ok(());
        };
        let result = self
            .panes
            .get_mut(&pane_id)
            .unwrap()
            .syntax_history
            .shrink(
                syntax,
                self.buffers[buffer_id].text(),
                &selection,
                mode,
                semantics,
            )?;
        match result {
            ShrinkResult::Restored {
                selection,
                mode,
                semantics,
            } => {
                self.active_mut().replace_selection(selection);
                self.active_mut().mark_selection_semantics(semantics);
                self.mode = mode;
            }
            ShrinkResult::Empty => self.status("no syntax expansion to shrink"),
            ShrinkResult::Reset(HistoryReset::Stale) => {
                self.status("syntax expansion history reset after syntax changed")
            }
            ShrinkResult::Reset(HistoryReset::Mismatch) => {
                self.status("syntax expansion history reset after selection changed")
            }
        }
        Ok(())
    }

    pub(super) fn transform_syntax_selection(
        &mut self,
        transform: SyntaxSelectionTransform,
    ) -> Result<()> {
        let buffer_id = self.active().buffer;
        let Some(syntax) = self.syntax[buffer_id].as_ref() else {
            self.mark_unavailable("syntax is unavailable for this buffer");
            return Ok(());
        };
        let selection = if self.active().selection_semantics() == SelectionSemantics::Runyte {
            self.vim_inclusive_to_half_open(self.active().selection.clone(), true)
        } else {
            self.active().selection.clone()
        };
        let selection = transform_selection(
            syntax,
            self.buffers[buffer_id].text(),
            &self.registry,
            &selection,
            transform,
        )?;
        let Some(selection) = selection else {
            self.status(match transform {
                SyntaxSelectionTransform::Expand => "no larger syntax selection",
                SyntaxSelectionTransform::Parent => "no syntax parent",
                SyntaxSelectionTransform::FirstNamedChild => "no syntax child",
                SyntaxSelectionTransform::PreviousNamedSibling => "no previous syntax sibling",
                SyntaxSelectionTransform::NextNamedSibling => "no next syntax sibling",
            });
            return Ok(());
        };
        self.install_inclusive_syntax_selection(selection);
        Ok(())
    }

    pub(super) fn select_syntax_object(
        &mut self,
        object: SyntaxObject,
        part: SyntaxObjectPart,
    ) -> Result<()> {
        let buffer_id = self.active().buffer;
        let Some(syntax) = self.syntax[buffer_id].as_ref() else {
            self.mark_unavailable("syntax is unavailable for this buffer");
            return Ok(());
        };
        let result = select_text_object(
            syntax,
            self.buffers[buffer_id].text(),
            &self.registry,
            &self.active().selection,
            object,
            part,
        );
        let selection = match result {
            Ok(Some(selection)) => selection,
            Ok(None) => {
                self.status(format!(
                    "no enclosing {} {} text object",
                    syntax_object_label(object),
                    syntax_object_part_label(part)
                ));
                return Ok(());
            }
            Err(
                error @ (SyntaxError::UnsupportedTextObject { .. }
                | SyntaxError::TextObjectQueryFailed { .. }),
            ) => {
                self.mark_unavailable(error.to_string());
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        self.install_inclusive_syntax_selection(selection);
        Ok(())
    }

    pub(super) fn select_delimiter(
        &mut self,
        pair: Option<DelimiterPair>,
        part: SyntaxObjectPart,
    ) -> Result<()> {
        let buffer_id = self.active().buffer;
        let Some(syntax) = self.syntax[buffer_id].as_ref() else {
            self.mark_unavailable("syntax is unavailable for this buffer");
            return Ok(());
        };
        let selection = select_delimiter(
            syntax,
            self.buffers[buffer_id].text(),
            &self.registry,
            &self.active().selection,
            pair,
            part,
        )?;
        let Some(selection) = selection else {
            self.status("no enclosing delimiter pair");
            return Ok(());
        };
        self.install_inclusive_syntax_selection(selection);
        Ok(())
    }

    /// Installs exact half-open syntax bounds using Runyte's visible inclusive
    /// cursor convention whenever the range contains text. An empty range must
    /// remain half-open: a Runyte point denotes the character under the caret
    /// rather than an empty operative span.
    fn install_inclusive_syntax_selection(&mut self, selection: Selection) {
        let nonempty = selection.ranges().iter().all(|range| !range.is_empty());
        let (selection, semantics) = if nonempty {
            (
                self.vim_half_open_to_inclusive(selection),
                SelectionSemantics::Runyte,
            )
        } else {
            (selection, SelectionSemantics::HalfOpen)
        };
        self.active_mut().replace_selection(selection);
        self.active_mut().mark_selection_semantics(semantics);
        self.mode = Mode::Select;
    }

    pub(super) fn navigate_syntax_object(
        &mut self,
        object: SyntaxObject,
        forward: bool,
    ) -> Result<()> {
        let buffer_id = self.active().buffer;
        let Some(syntax) = self.syntax[buffer_id].as_ref() else {
            self.mark_unavailable("syntax is unavailable for this buffer");
            return Ok(());
        };
        let result = navigate_text_object(
            syntax,
            self.buffers[buffer_id].text(),
            &self.registry,
            &self.active().selection,
            object,
            forward,
            self.mode,
        );
        let selection = match result {
            Ok(Some(selection)) => selection,
            Ok(None) => {
                self.status(format!(
                    "no {} syntax {}",
                    if forward { "next" } else { "previous" },
                    syntax_object_label(object)
                ));
                return Ok(());
            }
            Err(
                error @ (SyntaxError::UnsupportedTextObject { .. }
                | SyntaxError::TextObjectQueryFailed { .. }),
            ) => {
                self.mark_unavailable(error.to_string());
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        self.active_mut().replace_selection(selection);
        Ok(())
    }

    /// Opens the immediate Tree-sitter outline through the shared picker.
    ///
    /// Picker actions retain only Runyte-owned, revision-tagged ranges. The
    /// parser is consulted again on activation, so an intervening edit cannot
    /// turn an old result into a jump to unrelated text.
    pub(super) fn toggle_syntax_fold(&mut self) {
        let pane_id = self.active_pane;
        let buffer_id = self.panes[&pane_id].buffer;
        let cursor_row = self.panes[&pane_id].cursor(&self.buffers[buffer_id]).row;
        let Some(syntax) = self.syntax[buffer_id].as_ref() else {
            self.mark_unavailable("syntax folds are unavailable for this buffer");
            return;
        };
        let list = match syntax.folds(self.buffers[buffer_id].text(), &self.registry) {
            Ok(list) => list,
            Err(error) => {
                self.mark_unavailable(format!("syntax folds are unavailable: {error}"));
                return;
            }
        };
        let candidates = list
            .items
            .iter()
            .filter_map(|item| {
                let range = syntax
                    .resolve_fold_range(self.buffers[buffer_id].text(), item.range)
                    .ok()?;
                let from = self.buffers[buffer_id].position_of(range.from);
                let to = self.buffers[buffer_id].position_of(range.to);
                (cursor_row >= from.row && cursor_row < to.row).then_some((
                    item.range,
                    from.row,
                    to.row.saturating_sub(from.row),
                ))
            })
            .collect::<Vec<_>>();
        let collapsed = self.panes[&pane_id]
            .folds
            .collapsed
            .iter()
            .copied()
            .collect::<HashSet<_>>();
        // When nested ranges share one visible anchor, the widest collapsed
        // range is the effective fold the user can see. Opening a smaller
        // hidden child first would leave the screen unchanged.
        let candidate = candidates
            .iter()
            .copied()
            .filter(|(range, _, _)| collapsed.contains(range))
            .min_by_key(|(_, anchor, size)| {
                (usize::from(*anchor != cursor_row), std::cmp::Reverse(*size))
            })
            .or_else(|| {
                candidates
                    .into_iter()
                    .min_by_key(|(_, anchor, size)| (usize::from(*anchor != cursor_row), *size))
            });
        let Some((candidate, anchor_row, _)) = candidate else {
            self.status(format!(
                "no syntax fold at the cursor{}",
                fold_degradation_suffix(list.issues.len(), list.truncated)
            ));
            return;
        };
        let pane = self.panes.get_mut(&pane_id).unwrap();
        if let Some(index) = pane
            .folds
            .collapsed
            .iter()
            .position(|fold| *fold == candidate)
        {
            pane.folds.collapsed.remove(index);
            self.status(format!(
                "syntax fold opened{}",
                fold_degradation_suffix(list.issues.len(), list.truncated)
            ));
        } else {
            pane.folds.collapsed.push(candidate);
            let head = self.buffers[buffer_id].line_to_offset(anchor_row);
            self.panes
                .get_mut(&pane_id)
                .unwrap()
                .replace_selection(Selection::point(head));
            self.status(format!(
                "syntax fold closed{}",
                fold_degradation_suffix(list.issues.len(), list.truncated)
            ));
        }
    }

    pub(super) fn fold_all_syntax(&mut self) {
        let pane_id = self.active_pane;
        let buffer_id = self.panes[&pane_id].buffer;
        let Some(syntax) = self.syntax[buffer_id].as_ref() else {
            self.mark_unavailable("syntax folds are unavailable for this buffer");
            return;
        };
        let list = match syntax.folds(self.buffers[buffer_id].text(), &self.registry) {
            Ok(list) => list,
            Err(error) => {
                self.mark_unavailable(format!("syntax folds are unavailable: {error}"));
                return;
            }
        };
        let count = list.items.len();
        let issue_count = list.issues.len();
        let truncated = list.truncated;
        self.panes.get_mut(&pane_id).unwrap().folds.collapsed =
            list.items.into_iter().map(|item| item.range).collect();
        let cursor_row = self.cursor_position().row;
        if let Some(anchor) = self
            .resolved_folds(pane_id)
            .into_iter()
            .filter(|fold| fold.hides(cursor_row))
            .map(|fold| fold.anchor_row)
            .max()
        {
            let head = self.buffers[buffer_id].line_to_offset(anchor);
            self.panes
                .get_mut(&pane_id)
                .unwrap()
                .replace_selection(Selection::point(head));
        }
        self.status(format!(
            "folded {count} syntax region(s){}",
            fold_degradation_suffix(issue_count, truncated)
        ));
    }

    pub(super) fn unfold_all_syntax(&mut self) {
        let count = self.active().folds.collapsed.len();
        self.active_mut().folds.clear();
        self.status(format!("unfolded {count} syntax region(s)"));
    }

    pub(super) fn open_document_outline(&mut self) -> Result<()> {
        let buffer_id = self.active().buffer;
        let Some(syntax) = self.syntax[buffer_id].as_ref() else {
            self.mark_unavailable("document outline is unavailable for this buffer");
            return Ok(());
        };
        let outline = match syntax.outline(self.buffers[buffer_id].text(), &self.registry) {
            Ok(outline) => outline,
            Err(
                error @ (SyntaxError::UnsupportedOutline { .. }
                | SyntaxError::OutlineQueryFailed { .. }),
            ) => {
                self.mark_unavailable(error.to_string());
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        let status = outline_status(&outline);
        if outline.items.is_empty() {
            self.status(status.unwrap_or_else(|| "document outline is empty".to_owned()));
            return Ok(());
        }

        let items = outline
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                PickerItem::new(
                    item.name.to_string(),
                    outline_item_detail(&outline.items, index),
                    index,
                )
            })
            .collect();
        self.list_actions = outline
            .items
            .iter()
            .map(|item| ListAction::SyntaxOutline {
                buffer: buffer_id,
                target: item.target,
            })
            .collect();
        self.list = Some(ListPicker::new("Document outline", items).with_primary_action("jump"));
        if let Some(status) = status {
            self.status(status);
        }
        Ok(())
    }

    pub(crate) fn active_syntax_outline(&self) -> Result<Option<Outline>, SyntaxError> {
        let buffer_id = self.active().buffer;
        let Some(syntax) = self.syntax[buffer_id].as_ref() else {
            return Ok(None);
        };
        syntax
            .outline(self.buffers[buffer_id].text(), &self.registry)
            .map(Some)
    }

    pub(super) fn jump_to_syntax_outline(&mut self, buffer: usize, target: SyntaxSelectionRange) {
        if self.closed_buffers.contains(&buffer) {
            self.mark_unavailable("document outline is stale; its buffer was closed");
            return;
        }
        let Some(syntax) = self.syntax.get(buffer).and_then(Option::as_ref) else {
            self.mark_unavailable("document outline is stale; syntax is unavailable");
            return;
        };
        let range = match syntax.resolve_selection_range(self.buffers[buffer].text(), target) {
            Ok(range) => range,
            Err(SyntaxError::StaleRevision { .. } | SyntaxError::ForeignDocument) => {
                self.mark_unavailable("document outline is stale; reopen it");
                return;
            }
            Err(error) => {
                self.action_failed(error.to_string());
                return;
            }
        };
        self.push_jump();
        let selection = if range.to > range.from {
            Selection::single(Range::new(range.from, range.to - 1))
        } else {
            Selection::point(range.from)
        };
        let pane = self.active_mut();
        pane.retarget(buffer);
        pane.replace_selection(selection);
        pane.mark_selection_semantics(SelectionSemantics::Runyte);
        pane.preserve_scroll = false;
    }

    /// Moves each caret to the bracket matching the one under it.
    pub(super) fn match_bracket(&mut self) {
        let buffer_id = self.active().buffer;
        let Some(syntax) = self.syntax[buffer_id].as_ref() else {
            self.action_failed("no syntax tree for this buffer");
            return;
        };
        let text = self.buffers[buffer_id].text();
        let extend = self.mode == Mode::Select;
        let mut matched = false;
        let selection = self.active().selection.transform(|range| {
            let Some(target) = syntax.matching_bracket(text, range.head) else {
                return range;
            };
            matched = true;
            if extend {
                range.extend_to(target)
            } else {
                Range::point(target)
            }
        });
        if !matched {
            self.action_failed("no matching bracket");
            return;
        }
        let pane = self.active_mut();
        pane.replace_selection(selection);
        pane.preserve_scroll = false;
    }

    // -- Multi-selection commands -----------------------------------------

    pub(super) fn keep_primary_selection(&mut self) {
        let selection = self.active().selection.keep_primary();
        self.active_mut().replace_selection(selection);
        self.status("kept primary selection");
    }

    pub(super) fn remove_primary_selection(&mut self) {
        if self.active().selection.len() == 1 {
            self.action_failed("only one selection");
            return;
        }
        let selection = self.active().selection.remove_primary();
        self.active_mut().replace_selection(selection);
    }

    /// `(` and `)`: choose which of the ranges leads.
    ///
    /// Rotating does not change what the ranges are, so a pristine search
    /// result is still one: the presentation is re-stamped onto the new
    /// revision instead of being let go, and the matches stay drawn as
    /// matches with one of them primary. Every other selection change is a
    /// motion, where the ranges really have stopped being the search's.
    pub(super) fn rotate_selection(&mut self, forward: bool) {
        let pristine = self.pristine_search_selection(self.active_pane);
        let selection = self.active().selection.rotate(forward);
        self.active_mut().replace_selection(selection);
        if !pristine {
            return;
        }
        self.search_selection = Some(SearchSelectionPresentation {
            pane: self.active_pane,
            revision: self.active().selection_revision,
        });
        let index = self.active().selection.primary_index();
        let count = self.active().selection.len();
        self.status(format!(
            "match {}/{count} (all selected): {}",
            index + 1,
            self.search.pattern
        ));
    }

    /// Rotates the text held by the selections as one transaction.
    pub(super) fn rotate_selection_contents(&mut self, forward: bool) {
        let spans = self.operative_spans();
        if spans.len() < 2 {
            self.action_failed("rotating contents needs at least two selections");
            return;
        }
        let buffer = self.active_buffer();
        let values: Vec<String> = spans
            .iter()
            .map(|(from, to)| buffer.slice(*from, *to))
            .collect();
        let changes = spans
            .iter()
            .enumerate()
            .map(|(index, (from, to))| {
                let source = if forward {
                    (index + values.len() - 1) % values.len()
                } else {
                    (index + 1) % values.len()
                };
                Change::new(*from, *to, values[source].clone())
            })
            .collect();
        self.edit(Transaction::new(changes));
    }

    /// Leaves one bare cursor per selected line, at the line's start or end.
    ///
    /// This is the select-lines-then-type gesture: the cursors are collapsed
    /// rather than covering their lines, so the `i` or `a` that follows edits
    /// every line at once instead of replacing them.
    pub(super) fn split_selection_at_line_edges(&mut self, at_end: bool) {
        let buffer = self.active_buffer();
        let Some(selection) = self.active().selection.flat_transform(|range| {
            let (from, to) = operative_span(buffer, &range);
            if from == to {
                return vec![range];
            }
            let first = buffer.offset_to_row(from);
            let last = buffer.offset_to_row(to.saturating_sub(1));
            (first..=last)
                .map(|row| {
                    let start = buffer.line_to_offset(row);
                    Range::point(if at_end {
                        start + buffer.line_len(row)
                    } else {
                        start
                    })
                })
                .collect()
        }) else {
            return;
        };
        self.active_mut().replace_selection(selection);
        // Collapsed cursors have nothing to extend, and the point of the
        // command is to type at them, so it lands in NORMAL rather than SELECT.
        self.mode = Mode::Normal;
    }

    /// Adds a caret on the nearest row above or below every current caret that
    /// holds a character at the caret's own column.
    ///
    /// A row qualifies only when the column is *occupied*, not merely reachable:
    /// a row exactly as long as the column ends one character short, and putting
    /// the caret there would silently slide it left onto the last character.
    /// That shifted column then seeded the next `C`, so one short row bent every
    /// caret after it. Requiring an occupied column keeps the column exact, and
    /// rows that cannot hold it are skipped rather than approximated —
    /// `copy_selection_padded` is the command that widens them instead.
    pub(super) fn copy_selection(&mut self, down: bool) {
        let buffer = self.active_buffer();
        let last_row = buffer.last_row();
        let mut added = Vec::new();
        for range in self.active().selection.ranges() {
            let position = buffer.position_of(range.head);
            // Skip past rows too short to hold the column, rather than giving
            // up at the first one.
            let candidate = if down {
                (position.row + 1..=last_row).find(|row| buffer.line_len(*row) > position.col)
            } else {
                (0..position.row)
                    .rev()
                    .find(|row| buffer.line_len(*row) > position.col)
            };
            let Some(row) = candidate else {
                continue;
            };
            // In range by construction: the row is longer than the column, so
            // the caret lands on a character and needs no clamping.
            added.push(Range::point(buffer.line_to_offset(row) + position.col));
        }
        if added.is_empty() {
            self.action_failed("no room for another cursor");
            return;
        }
        let mut ranges = self.active().selection.ranges().to_vec();
        let primary = self.active().selection.primary_index();
        ranges.extend(added);
        self.active_mut()
            .replace_selection(Selection::new(ranges, primary));
    }

    /// Adds a caret on the adjacent row for every current caret, extending
    /// short rows with spaces so the new caret reaches the same display column.
    ///
    /// Padding is collected per row before any edit is applied, so several
    /// carets landing on one short row widen it once rather than compounding,
    /// and the whole thing stays a single undo step.
    pub(super) fn copy_selection_padded(&mut self, down: bool) {
        let buffer = self.active_buffer();
        let last_row = buffer.last_row();
        let tab_width = self.config.editor.tab_width;
        let mut additions = Vec::new();
        let mut padding: BTreeMap<usize, usize> = BTreeMap::new();

        for range in self.active().selection.ranges() {
            let position = buffer.position_of(range.head);
            let Some(row) = (if down {
                (position.row < last_row).then(|| position.row + 1)
            } else {
                position.row.checked_sub(1)
            }) else {
                continue;
            };
            let target = visual_column(&buffer.line_string(position.row), position.col, tab_width);
            let line = buffer.line_string(row);
            let width = visual_column(&line, line.chars().count(), tab_width);
            if width <= target {
                padding
                    .entry(row)
                    .and_modify(|existing| *existing = (*existing).max(target + 1 - width))
                    .or_insert(target + 1 - width);
            }
            additions.push((row, target));
        }

        if additions.is_empty() {
            self.action_failed("no room for another cursor");
            return;
        }

        let changes = padding
            .into_iter()
            .map(|(row, spaces)| {
                let end = buffer.line_to_offset(row) + buffer.line_len(row);
                Change::new(end, end, " ".repeat(spaces))
            })
            .collect();
        self.edit(Transaction::new(changes));

        let buffer = self.active_buffer();
        let mut ranges = self.active().selection.ranges().to_vec();
        let primary = self.active().selection.primary_index();
        ranges.extend(additions.into_iter().map(|(row, target)| {
            let line = buffer.line_string(row);
            let column = column_at_visual_column(&line, target, tab_width);
            Range::point(buffer.line_to_offset(row) + column)
        }));
        self.active_mut()
            .replace_selection(Selection::new(ranges, primary));
    }

    /// Keeps or removes ranges whose text matches `pattern`.
    ///
    /// The pattern is typed at the command's own prompt rather than inherited
    /// from the last search: filtering an existing multi-selection and finding
    /// text are separate questions, and tying them together meant the answer to
    /// one silently decided the other.
    pub(super) fn filter_selections(&mut self, keep: bool, pattern: &str) {
        let pattern = match Regex::new(pattern) {
            Ok(pattern) => pattern,
            Err(error) => {
                self.action_failed(format!("invalid regular expression: {error}"));
                return;
            }
        };
        let buffer = self.active_buffer();
        let filtered = self.active().selection.retain(|range| {
            let (from, to) = operative_span(buffer, range);
            pattern.is_match(&buffer.slice(from, to)) == keep
        });
        match filtered {
            Some(selection) => {
                let count = selection.len();
                self.active_mut().replace_selection(selection);
                self.status(format!("{count} selections"));
            }
            None => self.action_failed("that would remove every selection"),
        }
    }

    /// Trims leading and trailing whitespace from every selection.
    pub(super) fn trim_selections(&mut self) {
        let buffer = self.active_buffer();
        let selection = self.active().selection.transform(|range| {
            let (from, to) = operative_span(buffer, &range);
            let mut start = from;
            let mut end = to;
            while start < end && buffer.char_at(start).is_some_and(char::is_whitespace) {
                start += 1;
            }
            while end > start && buffer.char_at(end - 1).is_some_and(char::is_whitespace) {
                end -= 1;
            }
            if start >= end {
                // Nothing but whitespace: there is no span left to select, so
                // the range collapses to a caret rather than staying as it was.
                return Range::point(from);
            }
            Range::new(start, end.saturating_sub(1).max(start))
        });
        self.active_mut().replace_selection(selection);
    }

    /// Deletes trailing spaces and tabs from every line the selection touches.
    ///
    /// Unlike [`Self::trim_selections`] this changes the text, and it works a
    /// line at a time rather than a range at a time: what the selection picks
    /// out is which lines to trim, not which characters to remove. That is
    /// what makes `%` then `_` strip the whole buffer. Leading whitespace is
    /// left alone so indentation survives, which also means a line holding
    /// nothing but whitespace is emptied outright.
    pub(super) fn trim_trailing_whitespace_in_selection(&mut self) {
        let buffer = self.active_buffer();
        let half_open = matches!(
            self.active().selection_semantics(),
            SelectionSemantics::HalfOpen | SelectionSemantics::VimLinewise
        );
        let mut rows: Vec<usize> = self
            .active()
            .selection
            .ranges()
            .iter()
            .flat_map(|range| {
                let last = if half_open && !range.is_empty() {
                    range.to() - 1
                } else {
                    range.to()
                };
                buffer.offset_to_row(range.from())..=buffer.offset_to_row(last)
            })
            .collect();
        rows.sort_unstable();
        rows.dedup();
        let changes = trailing_whitespace_changes(buffer, rows.into_iter());
        let count = changes.len();
        if self.edit(Transaction::new(changes)) {
            self.status(format!(
                "trimmed trailing whitespace from {count} line{}",
                if count == 1 { "" } else { "s" }
            ));
        } else {
            self.status("no trailing whitespace in the selected lines");
        }
    }

    pub(super) fn hard_wrap_selections(&mut self, width: usize) {
        let buffer = self.active_buffer();
        let changes = self
            .operative_spans()
            .into_iter()
            .filter_map(|(from, to)| {
                let original = buffer.slice(from, to);
                let wrapped = crate::wrap::hard_wrap(&original, width);
                (wrapped != original).then(|| Change::new(from, to, wrapped))
            })
            .collect();
        if self.edit(Transaction::new(changes)) {
            self.status(format!("hard-wrapped selection to {width} characters"));
        } else {
            self.status("selection already fits the hard-wrap width");
        }
    }

    pub(super) fn reflow_selections(&mut self, width: usize) {
        let kind = match buffer_language(self.active_buffer(), &self.registry)
            .map(|language| self.registry.language_name(language))
        {
            Some("markdown") => crate::wrap::ReflowKind::Markdown,
            Some(_) => crate::wrap::ReflowKind::Source,
            None => crate::wrap::ReflowKind::Plain,
        };
        let buffer = self.active_buffer();
        let changes = self
            .operative_spans()
            .into_iter()
            .filter_map(|(from, to)| {
                let original = buffer.slice(from, to);
                let reflowed = crate::wrap::reflow(&original, width, kind);
                (reflowed != original).then(|| Change::new(from, to, reflowed))
            })
            .collect();
        if self.edit(Transaction::new(changes)) {
            self.status(format!("reflowed selection to {width} characters"));
        } else {
            self.status("selection is already reflowed");
        }
    }

    /// Replaces every line break inside every selection with `delimiter`.
    ///
    /// Acts strictly on what is selected, as the other selection-wide text
    /// transforms do: a selection covering one line has no break to remove and
    /// is left alone rather than reaching for the line below it. Deciding which
    /// breaks those are is this function's job rather than `join_lines`'s,
    /// because it needs the selection the span came from:
    ///
    /// - Under Runyte semantics a span carries no line terminator of its own, so
    ///   one at the end can only be the break before a selected empty last row,
    ///   and joining it is the whole point. The exception is a bare caret on an
    ///   empty row, whose span `operative_span` widens over that row's
    ///   terminator so `d` can delete the row; nothing is selected there to
    ///   join, so those ranges are dropped.
    /// - A half-open span, which a pointer drag produces, ends *at* the row
    ///   after it. Its final terminator belongs to the last selected row, and
    ///   removing it would pull up a row nobody selected, so it is held back
    ///   out of the change.
    pub(super) fn join_selections(&mut self, delimiter: &str) {
        let buffer = self.active_buffer();
        let half_open = matches!(
            self.active().selection_semantics(),
            SelectionSemantics::HalfOpen | SelectionSemantics::VimLinewise
        );
        // `operative_spans` maps the ranges one for one and in order, so the two
        // stay aligned.
        let changes = self
            .active()
            .selection
            .ranges()
            .iter()
            .zip(self.operative_spans())
            .filter(|(range, _)| !range.is_empty())
            .filter_map(|(_, (from, to))| {
                let to = if half_open {
                    without_trailing_line_terminator(buffer, from, to)
                } else {
                    to
                };
                let original = buffer.slice(from, to);
                let joined = crate::wrap::join_lines(&original, delimiter);
                (joined != original).then(|| Change::new(from, to, joined))
            })
            .collect();
        if self.edit(Transaction::new(changes)) {
            if delimiter.is_empty() {
                self.status("joined the selected lines");
            } else {
                self.status(format!("joined the selected lines with {delimiter:?}"));
            }
        } else {
            self.status("selection holds no line break to join");
        }
    }

    /// Aligns the columns of the table each selection covers.
    ///
    /// Alone among the selection-wide text transforms this widens each span to
    /// whole rows first, and folds together the spans that widening brings onto
    /// the same rows. A table row is a row from its opening `|` to its close, so
    /// a selection landing mid-row would otherwise be read as prose and refused
    /// — and `x` is not the only way people reach for a run of lines. Widening
    /// never reaches past the last selected row, so the rest of a table below
    /// the selection stays as it was.
    ///
    /// Nothing is edited unless every selection holds a table. A partial success
    /// would leave the person guessing which of their selections the status line
    /// was talking about, and the refusal is the whole point of the command
    /// noticing that a table is not there.
    pub(super) fn format_selected_tables(&mut self) {
        let tab_width = self.config.editor.tab_width;
        let formatted = {
            let buffer = self.active_buffer();
            merged_line_spans(buffer, self.operative_spans())
                .into_iter()
                .map(|(from, to)| {
                    let original = buffer.slice(from, to);
                    crate::table::format_table(&original, tab_width)
                        .map(|formatted| (from, to, original, formatted))
                })
                // Short-circuits on the first span that is not a table, so a
                // refusal costs nothing further.
                .collect::<Option<Vec<_>>>()
        };
        let Some(formatted) = formatted else {
            self.action_failed("no table detected in the selected lines");
            return;
        };
        let changes = formatted
            .into_iter()
            .filter(|(_, _, original, formatted)| formatted != original)
            .map(|(from, to, _, formatted)| Change::new(from, to, formatted))
            .collect();
        if self.edit(Transaction::new(changes)) {
            self.status("aligned the table columns");
        } else {
            self.status("the table columns are already aligned");
        }
    }

    /// Pads every selection's line so all selections start at the same column.
    pub(super) fn align_selections(&mut self) {
        let buffer = self.active_buffer();
        let columns: Vec<usize> = self
            .active()
            .selection
            .ranges()
            .iter()
            .map(|range| {
                let position = buffer.position_of(range.from());
                visual_column(
                    &buffer.line_string(position.row),
                    position.col,
                    self.config.editor.tab_width,
                )
            })
            .collect();
        let Some(target) = columns.iter().copied().max() else {
            return;
        };
        let changes: Vec<Change> = self
            .active()
            .selection
            .ranges()
            .iter()
            .zip(&columns)
            .filter(|(_, column)| **column < target)
            .map(|(range, column)| {
                let padding = " ".repeat(target - column);
                Change::new(range.from(), range.from(), padding)
            })
            .collect();
        if changes.is_empty() {
            return;
        }
        self.edit(Transaction::new(changes));
    }

    /// Concatenated text of every operative span, newline-separated when there
    /// is more than one selection.
    pub(super) fn selection_text(&self) -> String {
        let buffer = self.active_buffer();
        self.operative_spans()
            .into_iter()
            .map(|(from, to)| buffer.slice(from, to))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn directory_register(
        &self,
        mode: TransferMode,
        allow_caret: bool,
    ) -> Result<Option<DirectoryRegister>> {
        if !self.active_buffer().is_directory() {
            return Ok(None);
        }
        let buffer = self.active_buffer();
        let mut rows = Vec::new();
        for range in self.active().selection.ranges() {
            if range.is_empty() {
                if allow_caret {
                    rows.push(buffer.offset_to_row(range.head));
                }
                continue;
            }
            let (from, to) = operative_span(buffer, range);
            let first = buffer.offset_to_row(from);
            let last = buffer.offset_to_row(to.saturating_sub(1));
            if from != buffer.line_to_offset(first)
                || to != buffer.line_to_offset(last) + buffer.line_len(last)
            {
                return Ok(None);
            }
            rows.extend(first..=last);
        }
        rows.sort_unstable();
        rows.dedup();
        let entries = rows
            .into_iter()
            .map(|row| buffer.directory_transfer_at(row))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        Ok((!entries.is_empty()).then_some(DirectoryRegister { entries, mode }))
    }

    pub(super) fn delete_selection_or_char(
        &mut self,
        enter_insert: bool,
        transient_line_selection: bool,
    ) {
        let buffer_id = self.active().buffer;
        if enter_insert {
            self.buffers[buffer_id].begin_undo_group();
        }
        let linewise = transient_line_selection
            || self.active().selection_semantics() == SelectionSemantics::VimLinewise;
        let text = if linewise {
            self.line_register().text
        } else {
            self.selection_text()
        };
        let directory = match self.directory_register(TransferMode::Move, false) {
            Ok(directory) => directory,
            Err(error) => {
                self.action_failed(error.to_string());
                return;
            }
        };
        self.write_selected_register(Register {
            text,
            linewise,
            directory,
        });
        let buffer = self.active_buffer();
        let changes = self
            .operative_spans()
            .into_iter()
            .filter(|(from, to)| from < to)
            .map(|(from, to)| {
                if !transient_line_selection {
                    return Change::new(from, to, "");
                }
                let first_row = buffer.offset_to_row(from);
                let last_row = buffer.offset_to_row(to.saturating_sub(1));
                if last_row < buffer.last_row() {
                    Change::new(
                        buffer.line_to_offset(first_row),
                        buffer.line_to_offset(last_row + 1),
                        "",
                    )
                } else if first_row > 0 {
                    Change::new(
                        buffer.line_to_offset(first_row - 1) + buffer.line_len(first_row - 1),
                        buffer.len_chars(),
                        "",
                    )
                } else {
                    Change::new(0, buffer.len_chars(), "")
                }
            })
            .collect();
        self.edit(Transaction::new(changes));
        let selection = self.active().selection.collapse();
        self.active_mut().replace_selection(selection);
        self.mode = if enter_insert {
            Mode::Insert
        } else {
            Mode::Normal
        };
        self.normalize_buffer(buffer_id);
    }

    pub(super) fn replace_with_char(&mut self, replacement: char) {
        let buffer_id = self.active().buffer;
        let buffer = self.active_buffer();
        let changes = self
            .operative_spans()
            .into_iter()
            .filter(|(from, to)| from < to)
            .map(|(from, to)| {
                // Replace characters one for one, leaving line structure alone.
                let source = buffer.slice(from, to);
                let mut text = String::with_capacity(source.len());
                for (index, character) in source.chars().enumerate() {
                    if character == '\n'
                        || character == '\r' && buffer.char_at(from + index + 1) == Some('\n')
                    {
                        text.push(character);
                    } else {
                        text.push(replacement);
                    }
                }
                Change::new(from, to, text)
            })
            .collect();
        self.edit(Transaction::new(changes));
        self.mode = Mode::Normal;
        self.normalize_buffer(buffer_id);
    }

    pub(super) fn toggle_case(&mut self) {
        let buffer_id = self.active().buffer;
        let buffer = self.active_buffer();
        let changes = self
            .operative_spans()
            .into_iter()
            .filter(|(from, to)| from < to)
            .map(|(from, to)| {
                let text = buffer
                    .slice(from, to)
                    .chars()
                    .flat_map(|ch| {
                        if ch.is_lowercase() {
                            ch.to_uppercase().collect::<Vec<_>>()
                        } else {
                            ch.to_lowercase().collect::<Vec<_>>()
                        }
                    })
                    .collect::<String>();
                Change::new(from, to, text)
            })
            .collect();
        self.edit(Transaction::new(changes));
        self.mode = Mode::Normal;
        self.normalize_buffer(buffer_id);
    }

    pub(super) fn indent(&mut self, unindent: bool) {
        let buffer_id = self.active().buffer;
        let width = self.config.editor.tab_width.max(1);
        let buffer = &self.buffers[buffer_id];

        let mut rows: Vec<usize> = self
            .operative_spans()
            .into_iter()
            .flat_map(|(from, to)| {
                buffer.offset_to_row(from)..=buffer.offset_to_row(to.saturating_sub(1).max(from))
            })
            .collect();
        rows.sort_unstable();
        rows.dedup();

        let changes: Vec<Change> = rows
            .into_iter()
            .filter_map(|row| {
                let start = buffer.line_to_offset(row);
                if unindent {
                    let line = buffer.line_string(row);
                    let removed = if line.starts_with('\t') {
                        1
                    } else {
                        line.chars().take(width).take_while(|ch| *ch == ' ').count()
                    };
                    (removed > 0).then(|| Change::new(start, start + removed, ""))
                } else if buffer.line_len(row) > 0 {
                    Some(Change::new(start, start, " ".repeat(width)))
                } else {
                    None
                }
            })
            .collect();
        if changes.is_empty() {
            return;
        }
        self.edit(Transaction::new(changes));
        self.normalize_buffer(buffer_id);
    }

    /// Comments or uncomments every line the selection touches, using the
    /// marker the buffer's language declares.
    ///
    /// The marker goes at the shared minimum indent of the block rather than
    /// at each line's own indent, so a nested line keeps its relative
    /// indentation across the round trip. Blank lines are left alone in both
    /// directions: commenting one would only add trailing whitespace, and an
    /// empty line says nothing about whether the block is already commented.
    ///
    /// The gesture uncomments only when *every* non-blank line is already
    /// commented. A partly commented block therefore commutes to fully
    /// commented first, which is what makes a second press always the inverse
    /// of the first.
    ///
    /// Uncommenting consumes the marker and at most one space after it, so
    /// `// x` and `//x` both yield `x`. That rule is deliberately blunt about
    /// markers that merely begin with the language's own: under `//` a Rust
    /// doc comment `/// x` reads as commented and uncomments to `/ x`. Helix
    /// behaves the same way, and special-casing doc comments would trade one
    /// stray slash for a rule nobody can predict.
    ///
    /// A recognized shebang is left alone when it is the document's only
    /// language signal. Changing that first row would make an extensionless
    /// script lose both its syntax and its comment marker, so the next press
    /// could not invert the first one. Rows selected alongside it still
    /// toggle normally.
    pub(super) fn toggle_comments(&mut self) {
        /// One non-blank row the toggle acts on.
        struct CommentRow {
            row: usize,
            /// Character index of the row's first non-whitespace character.
            indent: usize,
            /// How many characters an uncomment would remove at `indent`, or
            /// `None` when the row is not commented.
            commented: Option<usize>,
        }

        let language = buffer_language(self.active_buffer(), &self.registry);
        let Some(marker) = language.and_then(|language| self.registry.line_comment(language))
        else {
            self.status(match language {
                Some(language) => format!(
                    "{} has no line comment",
                    self.registry.language_name(language)
                ),
                None => "no language for this buffer".to_owned(),
            });
            return;
        };

        let buffer_id = self.active().buffer;
        let buffer = &self.buffers[buffer_id];
        let preserve_shebang = buffer
            .path
            .as_deref()
            .and_then(|path| self.registry.language_for_path(path))
            .is_none()
            && buffer.line_string(0).starts_with("#!");

        let mut rows: Vec<usize> = self
            .operative_spans()
            .into_iter()
            .flat_map(|(from, to)| {
                buffer.offset_to_row(from)..=buffer.offset_to_row(to.saturating_sub(1).max(from))
            })
            .collect();
        rows.sort_unstable();
        rows.dedup();

        // Each surviving row keeps only what the edit needs: where its text
        // starts, and how much an uncomment would remove there. Blank rows
        // have no text start and drop out.
        let lines: Vec<CommentRow> = rows
            .into_iter()
            .filter_map(|row| {
                if row == 0 && preserve_shebang {
                    return None;
                }
                let line = buffer.line_string(row);
                let content = line.trim_start();
                if content.is_empty() {
                    return None;
                }
                let indent = line[..line.len() - content.len()].chars().count();
                let commented = content
                    .strip_prefix(marker)
                    .map(|rest| marker.chars().count() + usize::from(rest.starts_with(' ')));
                Some(CommentRow {
                    row,
                    indent,
                    commented,
                })
            })
            .collect();

        let Some(column) = lines.iter().map(|line| line.indent).min() else {
            return;
        };
        let uncomment = lines.iter().all(|line| line.commented.is_some());

        let changes: Vec<Change> = lines
            .iter()
            .map(|line| {
                let start = buffer.line_to_offset(line.row);
                if let Some(width) = line.commented.filter(|_| uncomment) {
                    let from = start + line.indent;
                    Change::new(from, from + width, "")
                } else {
                    let at = start + column;
                    Change::new(at, at, format!("{marker} "))
                }
            })
            .collect();
        self.edit(Transaction::new(changes));
        self.normalize_buffer(buffer_id);
    }

    pub(super) fn yank(&mut self, transient_line_selection: bool) {
        let register = self.yank_value(transient_line_selection);
        self.write_yanked_register(register);
    }

    /// `Y` takes the whole lines rather than the characters, so that the one
    /// key covers what `x y` covers without walking the selection through
    /// line mode first. The selection is left where it was: unlike `x`, this
    /// is a copy, not a way of choosing what to operate on next.
    pub(super) fn yank_line(&mut self) {
        let register = self.line_register();
        self.write_yanked_register(register);
    }

    fn write_yanked_register(&mut self, mut register: Register) {
        register.directory = match self.directory_register(TransferMode::Copy, true) {
            Ok(directory) => directory,
            Err(error) => {
                self.action_failed(error.to_string());
                return;
            }
        };
        // Yanking ends the gesture that chose the text, so Select mode hands
        // back to Normal whether the range spanned characters or only the
        // caret. The selection itself stays, which is what lets `P` paste at
        // its start.
        self.mode = Mode::Normal;
        self.write_selected_register(register);
        self.status("yanked");
    }

    pub(super) fn yank_value(&self, transient_line_selection: bool) -> Register {
        // An explorer caret names the entry on its row rather than the
        // character under it, which is how `directory_register` already reads
        // it. Yank the row so the text beside those entries agrees with them.
        if self.active_buffer().is_directory()
            && self.active().selection.ranges().iter().all(Range::is_empty)
        {
            return self.line_register();
        }
        let linewise = transient_line_selection
            || self.active().selection_semantics() == SelectionSemantics::VimLinewise;
        if linewise {
            return self.line_register();
        }
        Register {
            text: self.selection_text(),
            linewise: false,
            directory: None,
        }
    }

    /// Every row any range touches, terminated by newlines so the register
    /// pastes as whole lines.
    fn line_register(&self) -> Register {
        let buffer = self.active_buffer();
        let half_open = matches!(
            self.active().selection_semantics(),
            SelectionSemantics::HalfOpen | SelectionSemantics::VimLinewise
        );
        let mut rows: Vec<usize> = self
            .active()
            .selection
            .ranges()
            .iter()
            .flat_map(|range| {
                let last = if half_open && !range.is_empty() {
                    range.to() - 1
                } else {
                    range.to()
                };
                buffer.offset_to_row(range.from())..=buffer.offset_to_row(last)
            })
            .collect();
        rows.sort_unstable();
        rows.dedup();
        Register {
            text: rows
                .into_iter()
                .map(|row| {
                    format!(
                        "{}{}",
                        buffer.line_string(row),
                        line_terminator(buffer, row)
                            .map(|(terminator, _)| terminator)
                            .unwrap_or_else(|| preferred_line_ending(buffer, row))
                    )
                })
                .collect(),
            linewise: true,
            directory: None,
        }
    }

    pub(super) fn paste(&mut self, before: bool) {
        let register = self.read_selected_register();
        if register.text.is_empty() {
            return;
        }
        self.paste_register(&register, before);
    }

    pub(super) fn paste_register(&mut self, register: &Register, before: bool) {
        if self.active_buffer().is_directory()
            && let Some(directory) = &register.directory
        {
            if let Err(error) = self.paste_directory_register(directory, before) {
                self.action_failed(error.to_string());
            }
            return;
        }
        let buffer_id = self.active().buffer;
        let buffer = &self.buffers[buffer_id];
        let clipboard = register.text.clone();

        let changes = self
            .active()
            .selection
            .ranges()
            .iter()
            .map(|range| {
                let mut text = clipboard.clone();
                let at = if register.linewise {
                    let row = buffer.offset_to_row(range.head);
                    if before {
                        buffer.line_to_offset(row)
                    } else {
                        let line_end = buffer.line_to_offset(row) + buffer.line_len(row);
                        if line_end == buffer.len_chars() && line_end > 0 {
                            let terminator = if text.ends_with("\r\n") {
                                "\r\n"
                            } else {
                                preferred_line_ending(buffer, row)
                            };
                            text.insert_str(0, terminator);
                        }
                        if row < buffer.last_row() {
                            buffer.line_to_offset(row + 1)
                        } else {
                            buffer.len_chars()
                        }
                    }
                } else if before {
                    range.from()
                } else {
                    let row = buffer.offset_to_row(range.to());
                    let row_end = buffer.line_to_offset(row) + buffer.line_len(row);
                    (range.to() + 1).min(row_end)
                };
                Change::new(at, at, text)
            })
            .collect();
        self.edit(Transaction::new(changes));
        self.mode = Mode::Normal;
        self.normalize_buffer(buffer_id);
    }

    fn paste_directory_register(
        &mut self,
        register: &DirectoryRegister,
        before: bool,
    ) -> Result<()> {
        let buffer_id = self.active().buffer;
        let buffer = &self.buffers[buffer_id];
        let mut clipboard = register
            .entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        clipboard.push('\n');
        let clipboard_chars = clipboard.chars().count();
        let mut insertions = self
            .active()
            .selection
            .ranges()
            .iter()
            .map(|range| {
                let row = buffer.offset_to_row(range.head);
                if before {
                    buffer.line_to_offset(row)
                } else {
                    (buffer.line_to_offset(row) + buffer.line_len(row) + 1).min(buffer.len_chars())
                }
            })
            .collect::<Vec<_>>();
        insertions.sort_unstable();
        let changes = insertions
            .iter()
            .map(|at| Change::new(*at, *at, clipboard.clone()))
            .collect();
        if !self.edit(Transaction::new(changes)) {
            return Ok(());
        }
        for (index, at) in insertions.into_iter().enumerate() {
            let final_at = at + index * clipboard_chars;
            let start_row = self.buffers[buffer_id].offset_to_row(final_at);
            self.buffers[buffer_id].assign_directory_transfers(
                start_row,
                &register.entries,
                register.mode,
            )?;
        }
        self.mode = Mode::Normal;
        self.normalize_buffer(buffer_id);
        self.status(format!(
            "pasted {} entr{}; :write to review filesystem changes",
            register.entries.len(),
            if register.entries.len() == 1 {
                "y"
            } else {
                "ies"
            }
        ));
        Ok(())
    }

    pub(super) fn select_register(&mut self, name: char) {
        if name.is_control() {
            self.action_failed("register name must be printable");
            return;
        }
        self.selected_register = name;
        self.status(format!("register {name} selected"));
    }

    pub(super) fn write_selected_register(&mut self, value: Register) {
        let selected = std::mem::replace(&mut self.selected_register, '"');
        if selected == '_' {
            return;
        }
        self.registers.insert('"', value.clone());
        if selected == '"' {
            return;
        }
        if selected.is_ascii_uppercase() {
            let name = selected.to_ascii_lowercase();
            let target = self.registers.entry(name).or_default();
            target.text.push_str(&value.text);
            target.linewise |= value.linewise;
            match (&mut target.directory, value.directory) {
                (Some(target), Some(value)) if target.mode == value.mode => {
                    target.entries.extend(value.entries);
                }
                (None, None) => {}
                _ => target.directory = None,
            }
        } else {
            self.registers.insert(selected, value);
        }
    }

    pub(super) fn read_selected_register(&mut self) -> Register {
        let selected = std::mem::replace(&mut self.selected_register, '"');
        self.registers.get(&selected).cloned().unwrap_or_default()
    }
}

/// Existing line terminator and the offset of its `\n` token.
fn line_terminator(buffer: &Buffer, row: usize) -> Option<(&'static str, Offset)> {
    let end = buffer.line_to_offset(row) + buffer.line_len(row);
    if buffer.char_at(end) == Some('\r') && buffer.char_at(end + 1) == Some('\n') {
        Some(("\r\n", end + 1))
    } else if buffer.char_at(end) == Some('\n') {
        Some(("\n", end))
    } else {
        None
    }
}

/// Line ending to use for a newly inserted row near `row`.
fn preferred_line_ending(buffer: &Buffer, row: usize) -> &'static str {
    line_terminator(buffer, row)
        .or_else(|| {
            (0..row)
                .rev()
                .find_map(|candidate| line_terminator(buffer, candidate))
        })
        .or_else(|| {
            (row + 1..buffer.len_lines()).find_map(|candidate| line_terminator(buffer, candidate))
        })
        .map_or("\n", |(terminator, _)| terminator)
}

/// Expands deletions at either side of CRLF and unions any expansions that
/// now overlap. A selection or insert caret must never leave half a line
/// terminator behind.
fn crlf_safe_deletions(
    buffer: &Buffer,
    spans: impl IntoIterator<Item = (Offset, Offset)>,
) -> Vec<Change> {
    let mut spans = spans
        .into_iter()
        .filter(|(from, to)| from < to)
        .map(|(mut from, mut to)| {
            if from > 0
                && buffer.char_at(from) == Some('\n')
                && buffer.char_at(from - 1) == Some('\r')
            {
                from -= 1;
            }
            if buffer.char_at(to - 1) == Some('\r') && buffer.char_at(to) == Some('\n') {
                to += 1;
            }
            (from, to)
        })
        .collect::<Vec<_>>();
    spans.sort_unstable();

    let mut merged: Vec<(Offset, Offset)> = Vec::with_capacity(spans.len());
    for (from, to) in spans {
        if let Some((_, previous_to)) = merged.last_mut()
            && from <= *previous_to
        {
            *previous_to = (*previous_to).max(to);
        } else {
            merged.push((from, to));
        }
    }
    merged
        .into_iter()
        .map(|(from, to)| Change::new(from, to, ""))
        .collect()
}
