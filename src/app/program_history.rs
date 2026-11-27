// SPDX-License-Identifier: MPL-2.0

use super::{App, InputGrammar, Mode};

/// Undo ownership is per binding, not per event or root macro replay. Keep the
/// common single-buffer case inline; additional buffers are enrolled on edit.
#[derive(Clone, Debug)]
pub(super) struct ProgramUndo {
    first: Option<usize>,
    inherited_insert: Option<usize>,
    others: Vec<usize>,
}

impl ProgramUndo {
    fn contains(&self, buffer: usize) -> bool {
        self.first == Some(buffer) || self.others.contains(&buffer)
    }

    fn buffers(&self) -> impl Iterator<Item = usize> + '_ {
        self.first.into_iter().chain(self.others.iter().copied())
    }
}

impl App {
    pub(super) fn begin_program_undo(&mut self) {
        self.program_undo = Some(ProgramUndo {
            first: None,
            inherited_insert: (matches!(self.mode, Mode::Insert | Mode::Replace)
                && self.active_terminal().is_none())
            .then(|| self.active().buffer),
            others: Vec::new(),
        });
    }

    pub(super) fn finish_program_undo(&mut self, continue_insert: bool) {
        if let Some(owner) = self.program_undo.take() {
            self.close_program_undo(owner, continue_insert);
        }
    }

    fn close_program_undo(&mut self, owner: ProgramUndo, continue_insert: bool) {
        let insert_buffer = (continue_insert
            && matches!(self.mode, Mode::Insert | Mode::Replace)
            && self.active_terminal().is_none()
            && !self.active_buffer().is_read_only())
        .then(|| self.active().buffer);
        for buffer in owner.buffers() {
            if Some(buffer) != insert_buffer {
                self.buffers[buffer].commit_undo_group();
            }
        }
    }

    pub(super) fn finish_deferred_program_undo(&mut self) {
        let owners: Vec<_> = self
            .macro_replay
            .iter_mut()
            .flat_map(|replay| replay.commands.iter_mut())
            .filter_map(|command| command.program_undo.take())
            .collect();
        if !owners.is_empty() {
            self.finish_program_selection_history();
        }
        for owner in owners {
            self.close_program_undo(owner, false);
        }
    }

    pub(super) fn reset_program_input(&mut self) {
        self.finish_program_undo(false);
        self.grammar.reset();
    }

    /// Incidental mode/save boundaries yield to the binding's checkpoint.
    /// Explicit undo, redo and checkpoint actions call Buffer directly.
    pub(super) fn commit_edit_checkpoint(&mut self, buffer: usize) {
        if self.executing_program
            && self
                .program_undo
                .as_ref()
                .is_some_and(|owner| owner.contains(buffer))
        {
            return;
        }
        self.interrupt_program_edit(buffer);
        self.buffers[buffer].commit_undo_group();
    }

    pub(super) fn owns_program_buffer(&self, buffer: usize) -> bool {
        self.program_undo
            .as_ref()
            .is_some_and(|owner| owner.contains(buffer))
            || self.macro_replay.as_ref().is_some_and(|replay| {
                replay.commands.iter().any(|command| {
                    command
                        .program_undo
                        .as_ref()
                        .is_some_and(|owner| owner.contains(buffer))
                })
            })
    }

    pub(super) fn interrupt_program_edit(&mut self, buffer: usize) {
        if self.executing_program {
            return;
        }
        if self
            .program_undo
            .as_ref()
            .is_some_and(|owner| owner.contains(buffer))
        {
            self.reset_program_input();
            self.active_mut().program_selection_origin = None;
            self.active_mut().program_owner = None;
            self.status("binding cancelled: its buffer changed while waiting for a character");
        }
        if self.macro_replay.as_ref().is_some_and(|replay| {
            replay.commands.iter().any(|command| {
                command
                    .program_undo
                    .as_ref()
                    .is_some_and(|owner| owner.contains(buffer))
            })
        }) {
            self.cancel_macro_replay();
        }
    }

    pub(super) fn prepare_program_edit(&mut self, buffer: usize) {
        if !self.executing_program {
            self.interrupt_program_edit(buffer);
            return;
        }
        if let Some(owner) = self.program_undo.as_mut() {
            if !owner.contains(buffer) {
                if owner.inherited_insert != Some(buffer) {
                    self.buffers[buffer].commit_undo_group();
                }
                if owner.first.is_none() {
                    owner.first = Some(buffer);
                } else {
                    owner.others.push(buffer);
                }
            }
            // An explicit history action may have closed the previous group.
            self.buffers[buffer].begin_undo_group();
        }
    }
}
