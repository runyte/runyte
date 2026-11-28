// SPDX-License-Identifier: MPL-2.0

//! Bounded, revision-local selection history, independent of text undo.
//!
//! A buffer's history belongs to the pane showing it, because each pane keeps
//! its own selection in a shared buffer. A terminal's belongs to the terminal:
//! its review snapshot holds the one selection, whichever pane shows it, and
//! a new snapshot starts a new history the way a text edit does.

use std::collections::VecDeque;

use super::{App, Mode, Selection, SelectionSemantics};
use crate::terminal::TerminalId;

const MAX_STATES: usize = 128;
const MAX_RANGES: usize = 4096;
const MAX_BUFFERS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    selection: Selection,
    semantics: SelectionSemantics,
    mode: Mode,
    binding_mode: Option<String>,
    line_select: Option<super::binding_modes::TemporaryMode>,
}

/// What a selection is made in: a pane's buffer or a terminal's review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Subject {
    Buffer(usize),
    Terminal(TerminalId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Origin {
    pane: usize,
    subject: Subject,
    /// The buffer's text revision, or the terminal's review generation.
    revision: u64,
    state: State,
}

#[derive(Debug)]
pub(super) struct PromptOrigin {
    origin: Origin,
    cancelled: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct History {
    revision: u64,
    undo: VecDeque<State>,
    redo: VecDeque<State>,
}

impl History {
    fn check_revision(&mut self, revision: u64) {
        if self.revision != revision {
            self.undo.clear();
            self.redo.clear();
            self.revision = revision;
        }
    }

    fn bound(&mut self) {
        let mut ranges: usize = self
            .undo
            .iter()
            .chain(&self.redo)
            .map(|state| state.selection.len())
            .sum();
        while self.undo.len() + self.redo.len() > MAX_STATES || ranges > MAX_RANGES {
            let removed = self.undo.pop_front().or_else(|| self.redo.pop_front());
            let Some(removed) = removed else { break };
            ranges -= removed.selection.len();
        }
    }
}

impl App {
    pub(super) fn capture_selection_origin(&self) -> Option<Origin> {
        let pane = self.active();
        let mode = if self.mode == Mode::Command {
            self.prompt_origin_mode
        } else {
            self.mode
        };
        // History never re-enters an editing mode or reopens a prompt.
        let source_mode = mode;
        let mode = if mode == Mode::Select {
            Mode::Select
        } else {
            Mode::Normal
        };
        let binding_mode = (mode == source_mode)
            .then(|| {
                if self.mode == Mode::Command {
                    self.prompt_origin_binding_mode.clone()
                } else {
                    self.binding_mode_name()
                }
            })
            .flatten();
        if let Some(id) = self.active_terminal() {
            // Live output has no selection; only a review snapshot does.
            let (generation, selection) = self.terminals.get(id)?.review_selection_state()?;
            if selection.len() > MAX_RANGES {
                return None;
            }
            return Some(Origin {
                pane: self.active_pane,
                subject: Subject::Terminal(id),
                revision: generation,
                state: State {
                    selection: selection.clone(),
                    semantics: SelectionSemantics::default(),
                    mode,
                    binding_mode,
                    line_select: self.line_select.clone(),
                },
            });
        }
        if pane.terminal.is_some() || pane.selection.len() > MAX_RANGES {
            return None;
        }
        Some(Origin {
            pane: self.active_pane,
            subject: Subject::Buffer(pane.buffer),
            revision: self.active_buffer().revision(),
            state: State {
                selection: if matches!(source_mode, Mode::Insert | Mode::Replace) {
                    self.normal_mode_selection(
                        pane.buffer,
                        pane.selection.clone(),
                        pane.selection_semantics,
                        source_mode,
                    )
                } else {
                    pane.selection.clone()
                },
                semantics: pane.selection_semantics,
                mode,
                binding_mode,
                line_select: self.line_select.clone(),
            },
        })
    }

    /// Nested semantic execution belongs to its enclosing input action. This
    /// also groups counts while leaving individual replayed macro keys usable.
    pub(super) fn with_selection_action<T>(&mut self, action: impl FnOnce(&mut Self) -> T) -> T {
        let outer = self.selection_action_depth == 0;
        if outer {
            self.check_append_insert();
            self.finish_selection_drag();
            // Do not clone an oversized selection, or retain a history with
            // an unrecordable gap. Terminal visits leave buffer history alone.
            match self.active_terminal() {
                Some(id) => {
                    if self
                        .terminals
                        .get(id)
                        .is_some_and(|session| session.review_selection_count() > MAX_RANGES)
                    {
                        self.terminal_selection_history.remove(&id);
                    }
                }
                None if self.active().selection.len() > MAX_RANGES => {
                    let buffer = self.active().buffer;
                    self.active_mut().selection_history.remove(&buffer);
                }
                None => {}
            }
        }
        let origin_pane = self.active_pane;
        let before = (outer && self.selection_prompt_origin.is_none())
            .then(|| {
                self.active_mut()
                    .program_selection_origin
                    .clone()
                    .or_else(|| self.capture_selection_origin())
            })
            .flatten();
        let epoch = self.selection_history_epoch;
        self.selection_action_depth += 1;
        let result = action(self);
        self.selection_action_depth -= 1;
        if outer {
            self.follow_append_insert();
            if self.active_pane != origin_pane && self.mode != Mode::Command {
                self.selection_prompt_origin = None;
            }
        }
        if outer
            && self.active_pane != origin_pane
            && let Some(pane) = self.panes.get_mut(&origin_pane)
        {
            pane.program_selection_origin = None;
        }
        if outer && self.active_pane == origin_pane {
            if self.awaiting_character_command().is_some()
                || self
                    .macro_replay
                    .as_ref()
                    .is_some_and(|replay| !replay.commands.is_empty())
            {
                self.active_mut().program_selection_origin = before;
            } else if self.mode == Mode::Command {
                self.active_mut().program_selection_origin = None;
                // A prompt is part of the operation it collects, not a
                // sequence of selection changes. Retain one bounded origin.
                if self.selection_prompt_origin.is_none()
                    && let Some(origin) = before
                {
                    self.selection_prompt_origin = Some(PromptOrigin {
                        origin,
                        cancelled: false,
                    });
                }
            } else {
                self.active_mut().program_selection_origin = None;
                let before = match self.selection_prompt_origin.take() {
                    Some(prompt) if !prompt.cancelled => Some(prompt.origin),
                    Some(_) => None,
                    None => before,
                };
                if epoch == self.selection_history_epoch
                    && let Some(before) = before
                {
                    self.record_selection_action(before);
                }
            }
        }
        result
    }

    pub(super) fn finish_program_selection_history(&mut self) {
        if let Some(origin) = self.active_mut().program_selection_origin.take() {
            self.record_selection_action(origin);
        }
    }

    fn record_selection_action(&mut self, before: Origin) {
        let Some(after) = self.capture_selection_origin() else {
            // The action left a gap that cannot be recorded: an oversized
            // selection, or a terminal returned to live output.
            match before.subject {
                Subject::Buffer(buffer) => {
                    if let Some(pane) = self.panes.get_mut(&before.pane)
                        && pane.buffer == buffer
                        && pane.terminal.is_none()
                    {
                        pane.selection_history.remove(&buffer);
                    }
                }
                Subject::Terminal(id) => {
                    if self.active_pane == before.pane && self.active_terminal() == Some(id) {
                        self.terminal_selection_history.remove(&id);
                    }
                }
            }
            return;
        };
        if before.pane != after.pane || before.subject != after.subject {
            return;
        }
        let history = self.selection_history_entry(before.subject);
        history.check_revision(after.revision);
        // Opening/cancelling a prompt may change mode without changing the
        // selection. That is not a selection-history step.
        let changed = before.state.selection != after.state.selection
            || before.state.semantics != after.state.semantics
            || before.state.line_select != after.state.line_select;
        if before.revision == after.revision && changed {
            history.redo.clear();
            history.undo.push_back(before.state);
            history.bound();
        }
    }

    /// The history `subject` records into, making room for it if it is new.
    fn selection_history_entry(&mut self, subject: Subject) -> &mut History {
        match subject {
            Subject::Buffer(buffer) => {
                let pane = self.active_mut();
                if !pane.selection_history.contains_key(&buffer)
                    && pane.selection_history.len() >= MAX_BUFFERS
                {
                    // Prefer evicting a buffer least recently displayed by
                    // this pane.
                    let oldest = pane
                        .buffer_history
                        .iter()
                        .copied()
                        .find(|id| pane.selection_history.contains_key(id))
                        .or_else(|| pane.selection_history.keys().next().copied());
                    if let Some(oldest) = oldest {
                        pane.selection_history.remove(&oldest);
                    }
                }
                pane.selection_history.entry(buffer).or_default()
            }
            Subject::Terminal(id) => {
                let terminals = &self.terminals;
                let histories = &mut self.terminal_selection_history;
                if !histories.contains_key(&id) {
                    // Closed terminals never come back: ids are not reused.
                    histories.retain(|terminal, _| terminals.get(*terminal).is_some());
                    if histories.len() >= MAX_BUFFERS {
                        // Ids grow, so the smallest is the oldest terminal.
                        histories.pop_first();
                    }
                }
                histories.entry(id).or_default()
            }
        }
    }

    pub(super) fn cancel_selection_prompt(&mut self) {
        if let Some(prompt) = &mut self.selection_prompt_origin {
            prompt.cancelled = true;
        }
    }

    pub(super) fn finish_selection_drag(&mut self) {
        if let Some(before) = self.selection_drag_origin.take() {
            self.record_selection_action(before);
        }
    }

    pub(super) fn restore_selection_history(&mut self, redo: bool) {
        self.selection_history_epoch = self.selection_history_epoch.wrapping_add(1);
        let Some(current) = self.capture_selection_origin() else {
            if self
                .active_terminal()
                .and_then(|id| self.terminals.get(id))
                .is_some_and(|session| !session.reviewing())
            {
                // Live output has no selection to go back from.
                self.status(if redo {
                    "no later selection"
                } else {
                    "no earlier selection"
                });
            } else {
                self.status("selection history is unavailable in this view");
            }
            return;
        };
        let history = match current.subject {
            Subject::Buffer(buffer) => self.active_mut().selection_history.get_mut(&buffer),
            Subject::Terminal(id) => self.terminal_selection_history.get_mut(&id),
        };
        let Some(history) = history else {
            self.status(if redo {
                "no later selection"
            } else {
                "no earlier selection"
            });
            return;
        };
        history.check_revision(current.revision);
        let target = if redo {
            history.redo.pop_back()
        } else {
            history.undo.pop_back()
        };
        let Some(target) = target else {
            self.status(if redo {
                "no later selection"
            } else {
                "no earlier selection"
            });
            return;
        };
        if redo {
            history.undo.push_back(current.state);
        } else {
            history.redo.push_back(current.state);
        }
        history.bound();
        match current.subject {
            Subject::Buffer(_) => {
                let pane = self.active_mut();
                pane.replace_selection(target.selection);
                pane.mark_selection_semantics(target.semantics);
                pane.syntax_history.clear();
                pane.preserve_scroll = false;
                self.search_selection = None;
            }
            Subject::Terminal(id) => {
                let (_, rows) = self.pane_cells(self.active_pane);
                let scroll_offset = self.config.editor.scroll_offset;
                if let Some(session) = self.terminals.get_mut(id) {
                    // The generation was read just now, so this cannot miss.
                    session.restore_review_selection(current.revision, target.selection);
                    session.focus_review_selection(rows.max(1), scroll_offset);
                }
            }
        }
        self.restore_binding_mode(target.mode, target.binding_mode.as_deref());
        self.line_select = target.line_select;
        self.status(if redo {
            "selection redo"
        } else {
            "selection undo"
        });
    }
}
