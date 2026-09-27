// SPDX-License-Identifier: MPL-2.0

//! Bounded, revision-local selection history, independent of text undo.

use std::collections::VecDeque;

use super::{App, Mode, Selection, SelectionSemantics};

const MAX_STATES: usize = 128;
const MAX_RANGES: usize = 4096;
const MAX_BUFFERS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    selection: Selection,
    semantics: SelectionSemantics,
    mode: Mode,
    line_select: Option<Mode>,
}

#[derive(Clone, Debug)]
pub(super) struct Origin {
    pane: usize,
    buffer: usize,
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
        if pane.terminal.is_some() || pane.selection.len() > MAX_RANGES {
            return None;
        }
        Some(Origin {
            pane: self.active_pane,
            buffer: pane.buffer,
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
                line_select: self.line_select,
            },
        })
    }

    /// Nested semantic execution belongs to its enclosing input action. This
    /// also groups counts while leaving individual replayed macro keys usable.
    pub(super) fn with_selection_action<T>(&mut self, action: impl FnOnce(&mut Self) -> T) -> T {
        let outer = self.selection_action_depth == 0;
        if outer {
            self.finish_selection_drag();
            // Do not clone an oversized selection, or retain a history with
            // an unrecordable gap. Terminal visits leave buffer history alone.
            if self.active_terminal().is_none() && self.active().selection.len() > MAX_RANGES {
                let buffer = self.active().buffer;
                self.active_mut().selection_history.remove(&buffer);
            }
        }
        let before = (outer && self.selection_prompt_origin.is_none())
            .then(|| self.capture_selection_origin())
            .flatten();
        let epoch = self.selection_history_epoch;
        self.selection_action_depth += 1;
        let result = action(self);
        self.selection_action_depth -= 1;
        if outer {
            if self.mode == Mode::Command {
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

    fn record_selection_action(&mut self, before: Origin) {
        let Some(after) = self.capture_selection_origin() else {
            if let Some(pane) = self.panes.get_mut(&before.pane)
                && pane.buffer == before.buffer
                && pane.terminal.is_none()
            {
                pane.selection_history.remove(&before.buffer);
            }
            return;
        };
        if before.pane != after.pane || before.buffer != after.buffer {
            return;
        }
        let pane = self.active_mut();
        if !pane.selection_history.contains_key(&before.buffer)
            && pane.selection_history.len() >= MAX_BUFFERS
        {
            // Prefer evicting a buffer least recently displayed by this pane.
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
        let history = pane.selection_history.entry(before.buffer).or_default();
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
            self.status("selection history is unavailable in this view");
            return;
        };
        // The palette has already left Command mode before executing its
        // invocation. Its origin is the real selection state to redo.
        let current = self
            .selection_prompt_origin
            .as_ref()
            .filter(|prompt| {
                prompt.origin.pane == current.pane
                    && prompt.origin.buffer == current.buffer
                    && prompt.origin.revision == current.revision
            })
            .map_or(current, |prompt| prompt.origin.clone());
        let Some(history) = self.active_mut().selection_history.get_mut(&current.buffer) else {
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
        let pane = self.active_mut();
        pane.replace_selection(target.selection);
        pane.mark_selection_semantics(target.semantics);
        pane.syntax_history.clear();
        pane.preserve_scroll = false;
        self.mode = target.mode;
        self.line_select = target.line_select;
        self.search_selection = None;
        self.status(if redo {
            "selection redo"
        } else {
            "selection undo"
        });
    }
}
