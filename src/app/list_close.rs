// SPDX-License-Identifier: MPL-2.0
//! Captured close requests from destination lists. Selection changes never retarget a request.
use super::*;

#[derive(Clone, Debug)]
pub(super) enum Target {
    Buffer {
        id: usize,
        revision: u64,
        dirty: bool,
        save: bool,
    },
    Terminal {
        id: TerminalId,
        input: u64,
    },
    #[cfg(any(unix, windows))]
    Session(crate::workspace::WorkspaceSelection),
}

#[derive(Clone, Debug)]
pub(super) struct CloseTarget {
    target: Target,
    pub selected: usize,
    pub message: String,
}
impl CloseTarget {
    pub fn title(&self) -> &'static str {
        match self.target {
            Target::Buffer { .. } => "Close buffer",
            Target::Terminal { .. } => "Stop terminal",
            #[cfg(any(unix, windows))]
            Target::Session(_) => "Stop persistent session",
        }
    }
    pub fn labels(&self) -> &'static [&'static str] {
        match self.target {
            Target::Buffer {
                dirty: true,
                save: true,
                ..
            } => &["Cancel", "Save and close", "Discard changes and close"],
            Target::Buffer { dirty: true, .. } => &["Cancel", "Discard changes and close"],
            Target::Buffer { .. } => &["Cancel", "Close buffer"],
            Target::Terminal { .. } => &["Cancel", "Stop terminal"],
            #[cfg(any(unix, windows))]
            Target::Session(_) => &["Cancel", "Stop persistent session"],
        }
    }
}
impl App {
    pub(super) fn list_close_label(&self) -> Option<&'static str> {
        match self.selected_list_action()? {
            ListAction::Destination(OpenDestination::Buffer(_)) => Some("close buffer…"),
            ListAction::Destination(OpenDestination::Terminal(_)) => Some("stop terminal…"),
            #[cfg(any(unix, windows))]
            ListAction::Workspace(index)
                if self
                    .workspace_rows
                    .get(index)
                    .is_some_and(|row| row.running) =>
            {
                Some("stop session…")
            }
            _ => None,
        }
    }
    pub(super) fn request_list_close(&mut self) {
        match self.selected_list_action() {
            Some(ListAction::Destination(OpenDestination::Buffer(id))) => {
                self.request_buffer_close(id)
            }
            Some(ListAction::Destination(OpenDestination::Terminal(id))) => {
                self.request_terminal_close(id)
            }
            #[cfg(any(unix, windows))]
            Some(ListAction::Workspace(_)) => {
                self.open_session_actions();
                let Some(menu) = self.session_action_menu.take() else {
                    return;
                };
                self.request_session_close(menu.selection);
            }
            _ => {}
        }
    }
    #[cfg(any(unix, windows))]
    pub(super) fn request_session_close(
        &mut self,
        selection: crate::workspace::WorkspaceSelection,
    ) {
        let Ok(Some(index)) = self.workspace_row_index(&selection) else {
            return;
        };
        let row = &self.workspace_rows[index];
        if !row.running {
            self.status("this persistent session is already stopped");
            return;
        }
        self.show_list_close(Target::Session(selection), format!("Stop {}?\nUnsaved buffers, live terminal programs, and pending work may prevent stopping. Nothing is force-discarded.", row.display_name()));
        self.session_action_menu = None;
    }
    pub(super) fn request_buffer_close(&mut self, id: usize) {
        if self.closed_buffers.contains(&id) {
            return;
        }
        let Some(buffer) = self.buffers.get(id) else {
            return;
        };
        let target = Target::Buffer {
            id,
            revision: buffer.revision(),
            dirty: buffer.dirty,
            save: matches!(
                buffer.kind,
                BufferKind::File | BufferKind::Scratch | BufferKind::Provider(_)
            ),
        };
        let message = format!(
            "Close {}?{}",
            buffer.display_name(),
            if buffer.dirty {
                "\nThis buffer has unsaved changes. Discarding loses those edits; the file is not deleted."
            } else {
                "\nThe file is not deleted."
            }
        );
        self.buffer_action_menu = None;
        self.show_list_close(target, message);
    }
    pub(super) fn request_terminal_close(&mut self, id: TerminalId) {
        let Some(terminal) = self.terminals.get(id) else {
            return;
        };
        let confirm = terminal.close_needs_confirmation();
        let name = terminal.display_name();
        let input = terminal.input_generation();
        self.terminal_action_menu = None;
        if confirm {
            self.show_list_close(Target::Terminal { id, input }, format!("Stop {name}?\nThis terminates its shell and running programs and discards retained output. Unsaved work in those programs may be lost."));
        } else {
            self.finish_listed_terminal_close(id);
        }
    }
    fn finish_listed_terminal_close(&mut self, id: TerminalId) {
        self.close_terminal_id(id);
        self.refresh_navigator();
        if self.destination_scope() == Some(DestinationScope::Terminals)
            && self.terminals.is_empty()
        {
            self.list = None;
        }
    }
    fn show_list_close(&mut self, target: Target, message: String) {
        self.list_close = Some(CloseTarget {
            target,
            selected: 0,
            message,
        });
        self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
    }
    pub(super) fn handle_list_close_key(&mut self, key: KeyStroke) -> Result<()> {
        use crate::keymap::ui::{Action, Owner};
        // The opening key is never a confirmation, even when customized UI keys overlap.
        if self.list_binding_for(key) == Some(BindingTarget::Editor(EditorCommand::ListCloseTarget))
        {
            return Ok(());
        }
        match self.keymap.ui().lookup(Owner::ListClose, key) {
            Some(Action::Cancel) => {
                self.list_close = None;
            }
            Some(Action::Next | Action::Previous) => {
                let backwards =
                    self.keymap.ui().lookup(Owner::ListClose, key) == Some(Action::Previous);
                let close = self.list_close.as_mut().unwrap();
                let count = close.labels().len();
                close.selected = (close.selected + if backwards { count - 1 } else { 1 }) % count;
            }
            Some(Action::Accept) => {
                let close = self.list_close.take().unwrap();
                if close.selected == 0 {
                    return Ok(());
                }
                match close.target {
                    Target::Buffer {
                        id,
                        revision,
                        dirty,
                        save,
                    } => {
                        if self.closed_buffers.contains(&id)
                            || self.buffers[id].revision() != revision
                            || self.buffers[id].dirty != dirty
                        {
                            self.action_failed("buffer changed or closed; choose it again");
                            return Ok(());
                        }
                        if dirty && save && close.selected == 1 {
                            self.save_listed_buffer_and_close(id)?;
                        } else if dirty {
                            if self.refuse_running_merge_message(id) {
                                return Ok(());
                            }
                            self.close_buffer_discarding(id);
                        } else {
                            self.close_buffer_returning_from_commit(id);
                        }
                    }
                    Target::Terminal { id, input } => {
                        if self
                            .terminals
                            .get(id)
                            .is_some_and(|terminal| terminal.input_generation() == input)
                        {
                            self.finish_listed_terminal_close(id);
                        } else {
                            self.action_failed("terminal changed or closed; choose it again");
                        }
                    }
                    #[cfg(any(unix, windows))]
                    Target::Session(selection) => self.stop_selected_session(selection, false),
                }
                self.refresh_navigator();
            }
            _ => {}
        }
        Ok(())
    }
    fn save_listed_buffer_and_close(&mut self, id: usize) -> Result<()> {
        if self.document_mutation_pending(id) {
            self.action_failed("Document save is pending");
            return Ok(());
        }
        if self.buffers[id].provider().is_some() {
            self.list = None;
            self.visit_destination(OpenDestination::Buffer(id), false);
            self.queue_provider_save(
                id,
                Some(super::plugin_providers::ProviderSaveClose::CloseBuffer),
            );
        } else if self.buffers[id].path.is_none() && self.buffers[id].kind == BufferKind::Scratch {
            self.list = None;
            self.open_prompt(PromptKind::SaveClosePath {
                buffer: id,
                revision: self.buffers[id].revision(),
            });
        } else {
            self.host_save_buffer(id)?;
            self.close_buffer_returning_from_commit(id);
        }
        Ok(())
    }
    pub(super) fn save_close_path(&mut self, id: usize, revision: u64, path: &str) -> Result<()> {
        anyhow::ensure!(
            !self.closed_buffers.contains(&id) && self.buffers[id].revision() == revision,
            "buffer changed or closed; choose it again"
        );
        anyhow::ensure!(!path.trim().is_empty(), "a save path is required");
        self.save_buffer(id, Some(PathBuf::from(path)), false)?;
        if !self.buffers[id].dirty && !self.status_error {
            self.close_buffer_returning_from_commit(id);
        }
        Ok(())
    }
}
