// SPDX-License-Identifier: MPL-2.0

//! Search traversal, macros, clipboard actions, undo, and viewport scrolling.

// Application-module dependencies:
use super::{
    App, BindingScope, BindingTarget, DEFAULT_MACRO_REGISTER, EditorCommand, ListAction,
    ListPicker, Mode, PickerItem, Range, Register, Result, SearchMode, SearchQuery, SearchRegion,
    SearchSelectionPresentation, Selection, SelectionSemantics, Transaction, ViewAlignment,
    buffer_language, buffer_matches, move_projected_start_backward, next_offset, next_visible_row,
    offsets_after, offsets_before, operative_span, previous_offset, previous_visible_row,
    word_bounds,
};

impl App {
    /// The keys a command is reached by, for a message that has to name one.
    pub(super) fn binding_label(&self, command: EditorCommand) -> String {
        self.keymap
            .bindings_for_scope(Mode::Normal, BindingScope::Global)
            .find(|binding| binding.target == BindingTarget::Editor(command))
            .map_or_else(
                || format!(":{}", command.metadata().name),
                |binding| binding.sequence.to_string(),
            )
    }

    pub(super) fn clipboard_yank(&mut self) {
        let register = self.yank_value(false);
        match self.ports.clipboard().write(&register.text) {
            Ok(()) => self.status("yanked to system clipboard"),
            Err(error) => self.error(error.to_string()),
        }
    }

    pub(super) fn clipboard_paste(&mut self, before: bool) {
        match self.ports.clipboard().read() {
            Ok(text) if text.is_empty() => self.status("system clipboard is empty"),
            Ok(text) => {
                let linewise = text.ends_with('\n');
                self.paste_register(
                    &Register {
                        text,
                        linewise,
                        directory: None,
                    },
                    before,
                );
            }
            Err(error) => self.error(error.to_string()),
        }
    }

    /// The key that both starts and finishes an unnamed recording.
    ///
    /// One key for the whole gesture is the point of the macro namespace: a
    /// recording that has begun can always be ended by repeating what began
    /// it, without remembering a second, differently-named binding.
    pub(super) fn record_default_macro(&mut self) {
        if self.recording_macro.is_some() {
            self.stop_macro_recording();
        } else {
            self.start_macro_recording(DEFAULT_MACRO_REGISTER);
        }
    }

    pub(super) fn start_macro_recording(&mut self, register: char) {
        if register.is_control() {
            self.error("macro register must be printable");
            return;
        }
        if let Some(active) = self.recording_macro {
            self.error(format!(
                "already recording macro @{active}; {} stops it",
                self.macro_stop_hint()
            ));
            return;
        }
        self.macros.insert(register, Vec::new());
        self.macro_staging.clear();
        self.recording_macro = Some(register);
        self.status(format!(
            "recording macro @{register}; {} stops",
            self.macro_stop_hint()
        ));
    }

    fn macro_stop_hint(&self) -> &'static str {
        if self.grammar.kind() == crate::command::GrammarKind::Vim {
            "q"
        } else {
            "Space m m"
        }
    }

    pub(super) fn stop_macro_recording(&mut self) {
        self.macro_staging.clear();
        let Some(register) = self.recording_macro.take() else {
            self.status("no macro is being recorded");
            return;
        };
        let length = self.macros.get(&register).map_or(0, Vec::len);
        self.status(format!("recorded macro @{register}; {length} input(s)"));
    }

    /// Every macro that has been recorded, default one first.
    pub(super) fn open_macro_list(&mut self) {
        let mut registers = self.macros.keys().copied().collect::<Vec<_>>();
        registers.sort_unstable_by_key(|register| (*register != DEFAULT_MACRO_REGISTER, *register));
        if registers.is_empty() {
            self.status("no macros recorded");
            return;
        }
        let items = registers
            .iter()
            .enumerate()
            .map(|(index, register)| {
                let inputs = self.macros.get(register).map_or(0, Vec::len);
                let default = if *register == DEFAULT_MACRO_REGISTER {
                    "default · "
                } else {
                    ""
                };
                let recording = if self.recording_macro == Some(*register) {
                    " · recording"
                } else {
                    ""
                };
                PickerItem::new(
                    format!("@{register}"),
                    format!("{default}{inputs} input(s){recording}"),
                    index,
                )
            })
            .collect();
        self.list_actions = registers.into_iter().map(ListAction::Macro).collect();
        self.settings_view = None;
        self.list = Some(ListPicker::new("Macros", items).with_primary_action("replay"));
    }

    pub(super) fn replay_macro(&mut self, register: char, count: usize) -> Result<()> {
        let Some(inputs) = self.macros.get(&register).cloned() else {
            self.error(if register == DEFAULT_MACRO_REGISTER {
                "no default macro recorded; Space m m records one".to_owned()
            } else {
                format!("macro @{register} is empty")
            });
            return Ok(());
        };
        if self.replay_depth >= 16 {
            self.error("macro recursion limit reached");
            return Ok(());
        }
        self.replay_depth += 1;
        let outcome = (|| {
            for _ in 0..count {
                for input in &inputs {
                    self.handle_input(input.clone())?;
                }
            }
            Ok(())
        })();
        self.replay_depth -= 1;
        outcome
    }

    pub(super) fn undo(&mut self) {
        let buffer_id = self.active().buffer;
        let language_before = buffer_language(&self.buffers[buffer_id], &self.registry);
        if let Some(transactions) = self.buffers[buffer_id].undo_with_transactions() {
            self.map_transaction_views(buffer_id, &transactions);
            self.resync_replaced_buffer(buffer_id, language_before);
            self.normalize_buffer(buffer_id);
            self.status("undo");
            self.report_new_registry_errors();
        } else {
            self.status("nothing to undo");
        }
    }

    pub(super) fn redo(&mut self) {
        let buffer_id = self.active().buffer;
        let language_before = buffer_language(&self.buffers[buffer_id], &self.registry);
        if let Some(transactions) = self.buffers[buffer_id].redo_with_transactions() {
            self.map_transaction_views(buffer_id, &transactions);
            self.resync_replaced_buffer(buffer_id, language_before);
            self.normalize_buffer(buffer_id);
            self.status("redo");
            self.report_new_registry_errors();
        } else {
            self.status("nothing to redo");
        }
    }

    /// Keeps pane selections and remembered jumps attached to their logical
    /// text while an edit or history checkpoint applies transactions.
    pub(super) fn map_transaction_views(&mut self, buffer_id: usize, transactions: &[Transaction]) {
        for transaction in transactions {
            for pane in self.panes.values_mut() {
                // Every pane's history can hold this buffer, not just the
                // panes currently showing it.
                pane.jumps.map(buffer_id, transaction);
                if pane.buffer == buffer_id {
                    // Mapping changes coordinates, not the operation that
                    // created them, so each pane keeps its selection model.
                    pane.selection = pane.selection.map(transaction);
                    pane.preserve_scroll = false;
                }
            }
            // A scoped search must follow its region through edits for the same
            // reason a selection must: the offsets it stored describe text, not
            // positions, and `n` would otherwise wrap over the wrong span.
            if let Some(region) = &mut self.search.region
                && region.buffer == buffer_id
            {
                for span in &mut region.spans {
                    *span = span.map(transaction);
                }
            }
        }
    }

    /// The text the primary selection or the word under the caret stands for,
    /// which `*` searches for in both grammars.
    fn selection_search_pattern(&mut self) -> Option<String> {
        let buffer = self.active_buffer();
        let primary = self.active().selection.primary();
        let pattern = if primary.is_empty() {
            let (from, to) = word_bounds(buffer, primary.head);
            buffer.slice(from, to)
        } else {
            let (from, to) = operative_span(buffer, &primary);
            buffer.slice(from, to)
        };
        if pattern.is_empty() || pattern.contains('\n') {
            self.error("search selection must be non-empty and on one line");
            return None;
        }
        Some(pattern)
    }

    /// `*` in the Runyte grammar: select every occurrence of the word or
    /// selection under the caret.
    ///
    /// The pattern is taken literally and case-sensitively — the point of the
    /// gesture is "this exact identifier", so folding case would answer a
    /// question nobody asked. It searches the whole buffer because the
    /// selection is being read as the pattern here, not as a region.
    pub(super) fn search_selection(&mut self) {
        let Some(pattern) = self.selection_search_pattern() else {
            return;
        };
        self.commit_search(SearchQuery {
            pattern,
            mode: SearchMode::Sensitive,
            region: None,
            forward: true,
        });
    }

    /// `*` and `#` in the Vim grammar, which jump to one match at a time.
    pub(super) fn search_selection_direction(&mut self, reverse: bool, count: usize) {
        let Some(pattern) = self.selection_search_pattern() else {
            return;
        };
        self.search = SearchQuery {
            pattern,
            mode: SearchMode::Sensitive,
            region: None,
            forward: !reverse,
        };
        self.find_search(!reverse, true, count);
    }

    pub(super) fn repeat_search(&mut self, reverse: bool) {
        self.repeat_search_count(reverse, 1);
    }

    pub(super) fn repeat_search_count(&mut self, reverse: bool, count: usize) {
        if self.search.pattern.is_empty() {
            self.error("no previous search");
            return;
        }
        if self.grammar.kind() == crate::command::GrammarKind::Vim {
            let forward = if reverse {
                !self.search.forward
            } else {
                self.search.forward
            };
            // Repeats deliberately do not record: walking `n` through twenty
            // matches would bury the position the search started from.
            self.find_search(forward, false, count);
            return;
        }
        self.step_search(!reverse, count);
    }

    /// Every match of the committed query, in buffer order.
    ///
    /// A region from another buffer is ignored rather than honoured: the spans
    /// mean nothing there, and silently searching a stale region would be worse
    /// than searching the whole file.
    pub(super) fn search_matches(&self) -> Result<Vec<Range>, regex::Error> {
        let buffer = self.active_buffer();
        let spans = self
            .search
            .region
            .as_ref()
            .filter(|region| region.buffer == self.active().buffer)
            .map(|region| {
                region
                    .spans
                    .iter()
                    .map(|range| operative_span(buffer, range))
                    .collect::<Vec<_>>()
            });
        buffer_matches(
            buffer,
            &self.search.pattern,
            self.search.mode,
            spans.as_deref(),
        )
    }

    /// The spans a search started now would be confined to.
    ///
    /// A bare caret is a one-character range in this grammar, so requiring two
    /// characters is what separates "I selected something" from "my cursor is
    /// sitting somewhere". Successive searches narrow, by design: `x x x` then
    /// `s` then `/` walks from three lines to the matches inside them.
    pub(super) fn scoping_region(&self) -> Option<SearchRegion> {
        let buffer = self.active_buffer();
        let spans: Vec<Range> = self
            .active()
            .selection
            .ranges()
            .iter()
            .filter(|range| {
                let (from, to) = operative_span(buffer, range);
                to.saturating_sub(from) >= 2
            })
            .copied()
            .collect();
        (!spans.is_empty()).then(|| SearchRegion {
            buffer: self.active().buffer,
            spans,
        })
    }

    /// Runs a search and selects every match, leaving one cursor per match at
    /// the match's first character.
    ///
    /// The query is only committed when it finds something, so a typo does not
    /// cost the working `n`/`N` the person already had.
    pub(super) fn commit_search(&mut self, query: SearchQuery) {
        let previous = std::mem::replace(&mut self.search, query);
        let matches = match self.search_matches() {
            Ok(matches) => matches,
            Err(error) => {
                let pattern = std::mem::replace(&mut self.search, previous);
                self.error(format!(
                    "invalid regular expression: {error} in {}",
                    pattern.pattern
                ));
                return;
            }
        };
        if matches.is_empty() {
            let pattern = std::mem::replace(&mut self.search, previous);
            self.search_warning(format!("pattern not found: {}", pattern.pattern));
            return;
        }
        // The match at or after where the cursor already was becomes primary, so
        // `n` continues from the caret rather than from the top of the file.
        let current = self.active().selection.primary().from();
        let primary = matches
            .iter()
            .position(|range| range.from() >= current)
            .unwrap_or(0);
        let count = matches.len();
        self.push_jump();
        let pane = self.active_mut();
        pane.replace_selection(Selection::new(matches, primary));
        pane.preserve_scroll = false;
        let presentation = SearchSelectionPresentation {
            pane: self.active_pane,
            revision: self.active().selection_revision,
        };
        self.search_selection = Some(presentation);
        self.mode = Mode::Select;
        self.status(format!(
            "match {}/{count} (all selected): {}",
            primary + 1,
            self.search.pattern
        ));
    }

    /// `n` and `N`: select only the next or previous search match.
    ///
    /// A committed search initially selects every match for a direct batch
    /// edit. Cycling signals a single-match intent, so it reduces that
    /// multi-selection while retaining the query and its region for later
    /// repeats. A search over three selected lines can therefore never wrap
    /// out of them.
    fn step_search(&mut self, forward: bool, count: usize) {
        let matches = match self.search_matches() {
            Ok(matches) => matches,
            Err(error) => {
                self.error(format!("invalid regular expression: {error}"));
                return;
            }
        };
        if matches.is_empty() {
            self.search_warning(format!("pattern not found: {}", self.search.pattern));
            return;
        }
        let current = self.active().selection.primary().from();
        let first = if forward {
            matches
                .iter()
                .position(|range| range.from() > current)
                .unwrap_or(0)
        } else {
            matches
                .iter()
                .rposition(|range| range.from() < current)
                .unwrap_or(matches.len() - 1)
        };
        let distance = count.saturating_sub(1) % matches.len();
        let index = if forward {
            (first + distance) % matches.len()
        } else {
            (first + matches.len() - distance) % matches.len()
        };
        let total = matches.len();
        let pane = self.active_mut();
        pane.replace_selection(Selection::single(matches[index]));
        pane.preserve_scroll = false;
        let presentation = SearchSelectionPresentation {
            pane: self.active_pane,
            revision: self.active().selection_revision,
        };
        self.search_selection = Some(presentation);
        self.mode = Mode::Select;
        self.status(format!(
            "match {}/{total}: {}",
            index + 1,
            self.search.pattern
        ));
    }

    /// Moves to a single match, the way the Vim grammar's `/`, `?`, `n`, `N`,
    /// `*`, and `#` do. `record` distinguishes starting a search, which is a
    /// jump, from stepping through its results, which is not.
    ///
    /// This deliberately keeps its own literal line scan rather than sharing
    /// [`buffer_matches`]: the Runyte grammar's flavours and multicursor result
    /// are not what Vim's search means, and folding the two together would drag
    /// one grammar's semantics into the other.
    pub(super) fn find_search(&mut self, forward: bool, record: bool, count: usize) {
        if self.search.pattern.is_empty() {
            return;
        }
        let buffer = self.active_buffer();
        let pattern_len = self.search.pattern.chars().count();
        let mut matches = Vec::new();
        for row in 0..buffer.len_lines() {
            let line = buffer.line_string(row);
            let start = buffer.line_to_offset(row);
            for (byte, _) in line.match_indices(&self.search.pattern) {
                let from = start + line[..byte].chars().count();
                matches.push((from, from + pattern_len.saturating_sub(1)));
            }
        }
        if matches.is_empty() {
            self.search_warning(format!("pattern not found: {}", self.search.pattern));
            return;
        }

        let current = self.active().selection.primary().from();
        let first_index = if forward {
            matches
                .iter()
                .position(|(from, _)| *from > current)
                .unwrap_or(0)
        } else {
            matches
                .iter()
                .rposition(|(from, _)| *from < current)
                .unwrap_or(matches.len() - 1)
        };
        let distance = count.saturating_sub(1) % matches.len();
        let found_index = if forward {
            (first_index + distance) % matches.len()
        } else {
            (first_index + matches.len() - distance) % matches.len()
        };
        let found = matches[found_index];

        if record {
            self.push_jump();
        }
        let vim = self.grammar.kind() == crate::command::GrammarKind::Vim;
        let vim_visual =
            vim && (self.mode == Mode::Select || record && self.prompt_origin_mode == Mode::Select);
        if vim_visual
            && matches!(
                self.active().selection_semantics(),
                SelectionSemantics::HalfOpen | SelectionSemantics::VimLinewise
            )
        {
            let selection = self.vim_half_open_to_inclusive(self.active().selection.clone());
            self.active_mut().replace_selection(selection);
        }
        let selection = if vim_visual {
            self.active()
                .selection
                .transform(|range| range.extend_to(found.1))
        } else if vim {
            Selection::point(found.0)
        } else {
            Selection::single(Range::new(found.0, found.1))
        };
        let pane = self.active_mut();
        pane.replace_selection(selection);
        pane.preserve_scroll = false;
        self.mode = if vim {
            if vim_visual {
                Mode::Select
            } else {
                Mode::Normal
            }
        } else {
            Mode::Select
        };
        if vim {
            let selection =
                self.vim_inclusive_to_half_open(self.active().selection.clone(), vim_visual);
            self.active_mut().replace_selection(selection);
            self.active_mut()
                .mark_selection_semantics(SelectionSemantics::HalfOpen);
        }
        self.status(format!("search: {}", self.search.pattern));
    }

    pub(super) fn find_character(&mut self, character: char, forward: bool, till: bool) {
        let buffer = self.active_buffer();
        let extend = self.mode == Mode::Select;
        let mut missed = false;
        let selection = self.active().selection.transform(|range| {
            let found = if forward {
                offsets_after(buffer, range.head)
                    .find(|offset| buffer.char_at(*offset) == Some(character))
            } else {
                offsets_before(buffer, range.head)
                    .find(|offset| buffer.char_at(*offset) == Some(character))
            };
            let Some(mut target) = found else {
                missed = true;
                return range;
            };
            if till {
                target = if forward {
                    previous_offset(buffer, target).unwrap_or(range.head)
                } else {
                    next_offset(buffer, target).unwrap_or(range.head)
                };
            }
            if extend {
                range.extend_to(target)
            } else {
                Range::point(target)
            }
        });
        if missed {
            self.error(format!("character not found: {character}"));
        }
        let pane = self.active_mut();
        pane.replace_selection(selection);
        pane.preserve_scroll = false;
    }

    pub(super) fn viewport_height(&self) -> usize {
        self.areas
            .get(&self.active_pane)
            .map_or(20, |area| area.height.saturating_sub(2) as usize)
            .max(1)
    }

    fn viewport_width(&self) -> usize {
        self.areas
            .get(&self.active_pane)
            .map_or(80, |area| area.width.saturating_sub(2) as usize)
            .max(1)
    }

    pub(super) fn align_view(&mut self, alignment: ViewAlignment) {
        let cursor = self.cursor_position();
        let height = self.viewport_height();
        let pane_id = self.active_pane;
        let width = self.panes[&pane_id].wrap_width.max(1);
        let buffer_id = self.panes[&pane_id].buffer;
        let soft_wrap = self.pane_soft_wrap(pane_id);
        let folds = self.resolved_folds(pane_id);
        {
            let line = self.buffers[buffer_id].line_string(cursor.row);
            let segment = if soft_wrap {
                crate::wrap::segment_index(&line, cursor.col, width, self.config.editor.tab_width)
            } else {
                0
            };
            let amount = match alignment {
                ViewAlignment::Top => 0,
                ViewAlignment::Center => height / 2,
                ViewAlignment::Bottom => height.saturating_sub(1),
            };
            let (row, segment) = move_projected_start_backward(
                &self.buffers[buffer_id],
                &folds,
                cursor.row,
                segment,
                amount,
                width,
                self.config.editor.tab_width,
                soft_wrap,
            );
            let pane = self.active_mut();
            pane.scroll_row = row;
            pane.scroll_wrap = segment;
            pane.scroll_col = 0;
            pane.preserve_scroll = true;
        }
    }

    pub(super) fn align_view_middle(&mut self) {
        let cursor_col = self.cursor_position().col;
        let width = self.viewport_width();
        self.active_mut().scroll_col = cursor_col.saturating_sub(width / 2);
    }

    pub(super) fn scroll_view(&mut self, direction: i32) {
        self.scroll_pane(self.active_pane, direction);
    }

    pub(super) fn scroll_pane(&mut self, pane_id: usize, direction: i32) {
        let buffer_id = self.panes[&pane_id].buffer;
        let width = self.panes[&pane_id].wrap_width.max(1);
        let row = self.panes[&pane_id].scroll_row;
        let segment = self.panes[&pane_id].scroll_wrap;
        let soft_wrap = self.pane_soft_wrap(pane_id);
        let folds = self.resolved_folds(pane_id);
        if soft_wrap {
            let segment_count = crate::wrap::segments(
                &self.buffers[buffer_id].line_string(row),
                width,
                self.config.editor.tab_width,
            )
            .len();
            let previous_row = previous_visible_row(&folds, row);
            let previous_segment = (row > 0).then(|| {
                crate::wrap::segments(
                    &self.buffers[buffer_id].line_string(previous_row),
                    width,
                    self.config.editor.tab_width,
                )
                .len()
                .saturating_sub(1)
            });
            let last_row = self.buffers[buffer_id].last_row();
            let pane = self.active_mut();
            if direction < 0 {
                if segment > 0 {
                    pane.scroll_wrap -= 1;
                } else if row > 0 {
                    pane.scroll_row = previous_row;
                    pane.scroll_wrap = previous_segment.unwrap_or(0);
                }
            } else if segment + 1 < segment_count {
                pane.scroll_wrap += 1;
            } else if row < last_row {
                pane.scroll_row = next_visible_row(&folds, row, last_row);
                pane.scroll_wrap = 0;
            }
            pane.scroll_col = 0;
            pane.preserve_scroll = true;
            return;
        }
        let last_row = self.buffers[buffer_id].last_row();
        let pane = self.panes.get_mut(&pane_id).unwrap();
        if direction < 0 {
            pane.scroll_row = previous_visible_row(&folds, pane.scroll_row);
        } else {
            pane.scroll_row = next_visible_row(&folds, pane.scroll_row, last_row);
        }
        pane.preserve_scroll = true;
    }
}
