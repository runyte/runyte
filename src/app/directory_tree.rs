// SPDX-License-Identifier: MPL-2.0

use super::{
    App, ApplyReport, EditorCommand, EntryKind, FsConfirmation, FsConfirmationOrigin, FsOperation,
    Mode, PromptKind, TreePromptAction,
};
use anyhow::Result;

impl App {
    /// Handles sidebar commands before the backing pane's terminal or buffer
    /// can interpret them. The ordinary pane remains the file destination.
    pub(super) fn handle_directory_tree_command(&mut self, command: EditorCommand) -> Result<bool> {
        use EditorCommand as Command;
        let show_hidden = self.config.editor.show_hidden_files;
        match command {
            Command::ToggleDirectoryTree => {
                if self.directory_tree.visible {
                    self.directory_tree.visible = false;
                    self.leave_directory_tree();
                } else {
                    self.directory_tree.show(show_hidden);
                }
            }
            Command::FocusDirectoryTree => {
                self.maximized = None;
                self.directory_tree.show(show_hidden);
                let path = self.active_buffer().path.clone().or_else(|| {
                    self.active_buffer()
                        .markdown_render_source()
                        .and_then(|source| self.buffers.get(source))
                        .and_then(|buffer| buffer.path.clone())
                });
                if let Some(path) = path {
                    if let Err(error) = self.directory_tree.reveal(&path, show_hidden) {
                        self.directory_tree.selected = self.directory_tree.root.clone();
                        self.status(error.to_string());
                    }
                } else {
                    self.directory_tree.selected = self.directory_tree.root.clone();
                }
                if !self.directory_tree.focused {
                    self.directory_tree_previous_mode = self.mode;
                    self.mode = Mode::Normal;
                    self.directory_tree.focused = true;
                }
            }
            Command::DirectoryTreeClose => self.leave_directory_tree(),
            Command::DirectoryTreeUp => self.directory_tree.select_relative(-1),
            Command::DirectoryTreeDown => self.directory_tree.select_relative(1),
            Command::DirectoryTreeLeft => self.directory_tree.collapse_or_parent(),
            Command::DirectoryTreeRight => self.directory_tree.expand_or_child(show_hidden),
            Command::DirectoryTreeFirst => self.directory_tree.select_first(),
            Command::DirectoryTreeLast => self.directory_tree.select_last(),
            Command::DirectoryTreePageUp => self
                .directory_tree
                .select_relative(-(self.directory_tree.viewport_rows as isize)),
            Command::DirectoryTreePageDown => self
                .directory_tree
                .select_relative(self.directory_tree.viewport_rows as isize),
            Command::DirectoryTreeOpen => {
                if self.directory_tree.kind(&self.directory_tree.selected)
                    == Some(EntryKind::Directory)
                {
                    self.directory_tree.toggle_selected(show_hidden);
                } else {
                    let path = self.directory_tree.selected.clone();
                    self.open_file(path)?;
                    self.directory_tree.focused = false;
                    if self.prompt_kind != PromptKind::ExternalProgram {
                        self.mode = Mode::Normal;
                    }
                }
            }
            Command::DirectoryTreeRefresh => {
                let selected = self.directory_tree.selected.clone();
                let path = if self.directory_tree.kind(&selected) == Some(EntryKind::Directory) {
                    selected
                } else {
                    selected
                        .parent()
                        .unwrap_or(&self.directory_tree.root)
                        .to_path_buf()
                };
                self.directory_tree.refresh(path, show_hidden);
            }
            Command::DirectoryTreeNew => {
                let selected = self.directory_tree.selected.clone();
                let parent = if self.directory_tree.kind(&selected) == Some(EntryKind::Directory) {
                    selected
                } else {
                    selected
                        .parent()
                        .unwrap_or(&self.directory_tree.root)
                        .to_path_buf()
                };
                self.directory_tree_prompt_target = Some((parent, self.directory_tree.revision));
                self.open_prompt(PromptKind::DirectoryTreeAction(TreePromptAction::New));
            }
            Command::DirectoryTreeRename | Command::DirectoryTreeMove => {
                let selected = self.directory_tree.selected.clone();
                if selected == self.directory_tree.root {
                    self.action_failed("the workspace root cannot be renamed or moved");
                    return Ok(true);
                }
                self.directory_tree_prompt_target =
                    Some((selected.clone(), self.directory_tree.revision));
                if command == Command::DirectoryTreeRename {
                    let displayed_path = self
                        .directory_tree
                        .planned_destination(&selected)
                        .unwrap_or(&selected);
                    let name = displayed_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    self.open_prompt_with_value(
                        PromptKind::DirectoryTreeAction(TreePromptAction::Rename),
                        name,
                    );
                } else {
                    self.open_prompt(PromptKind::DirectoryTreeAction(TreePromptAction::Move));
                }
            }
            Command::DirectoryTreeDelete => {
                let selected = self.directory_tree.selected.clone();
                if selected == self.directory_tree.root {
                    self.action_failed("the workspace root cannot be deleted");
                } else if let Err(error) = self.directory_tree.stage_delete(&selected, show_hidden)
                {
                    self.action_failed(error.to_string());
                } else {
                    self.status("deletion staged; Tab p reviews the plan");
                }
            }
            Command::DirectoryTreeReview => {
                if self.directory_tree.pending_count() == 0 {
                    self.status("directory tree has no pending changes");
                } else {
                    match self.directory_tree.build_plan() {
                        Ok(plan) => {
                            self.fs_confirmation = Some(FsConfirmation {
                                origin: FsConfirmationOrigin::DirectoryTree {
                                    revision: self.directory_tree.revision,
                                },
                                plan,
                                selected: 0,
                            });
                            self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
                        }
                        Err(error) => self.action_failed(error.to_string()),
                    }
                }
            }
            Command::DirectoryTreeUndo => {
                if self.directory_tree.undo() {
                    self.status("last staged change undone");
                } else {
                    self.status("no staged change to undo");
                }
            }
            Command::DirectoryTreeClear => {
                if self.directory_tree.pending_count() == 0 {
                    self.status("directory tree has no pending changes");
                } else {
                    self.directory_tree_discard_confirmation = true;
                }
            }
            Command::ShowHelp if self.directory_tree.focused => {
                self.open_help();
                self.leave_directory_tree();
                self.mode = Mode::Normal;
            }
            _ if self.directory_tree.focused => {
                // Global bindings remain visible for discovery, but document
                // edits and terminal commands cannot reach the covered pane.
                if matches!(
                    command,
                    Command::ToggleFullscreen
                        | Command::ToggleZen
                        | Command::OpenCommandPalette
                        | Command::FocusWindowLeft
                        | Command::FocusWindowRight
                        | Command::FocusWindowUp
                        | Command::FocusWindowDown
                        | Command::NextWindow
                ) {
                    if matches!(command, Command::ToggleFullscreen | Command::ToggleZen) {
                        self.leave_directory_tree();
                    }
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(super) fn leave_directory_tree(&mut self) {
        if self.directory_tree.focused {
            self.directory_tree.focused = false;
            if self.mode != Mode::Command {
                self.mode = self.directory_tree_previous_mode;
            }
        }
    }

    pub(super) fn accept_directory_tree_prompt(
        &mut self,
        action: TreePromptAction,
        target: (std::path::PathBuf, u64),
        value: &str,
    ) {
        if target.1 != self.directory_tree.revision {
            self.action_failed("pending directory tree plan changed; choose the action again");
            return;
        }
        let show_hidden = self.config.editor.show_hidden_files;
        let result = match action {
            TreePromptAction::New => {
                self.directory_tree
                    .stage_create(&target.0, value, show_hidden)
            }
            TreePromptAction::Rename => {
                self.directory_tree
                    .stage_rename(&target.0, value, show_hidden)
            }
            TreePromptAction::Move => self
                .directory_tree
                .stage_move(&target.0, value, show_hidden),
        };
        match result {
            Ok(path) => self.status(format!(
                "staged {} · Tab p reviews the plan",
                path.display()
            )),
            Err(error) => self.action_failed(error.to_string()),
        }
    }

    pub(super) fn refresh_directory_tree_after_report(
        &mut self,
        root: &std::path::Path,
        report: &ApplyReport,
    ) {
        let show_hidden = self.config.editor.show_hidden_files;
        let mut selected_destination = None;
        for operation in &report.applied {
            let source = match operation {
                FsOperation::Delete { path, .. } => Some(path),
                FsOperation::Rename { from, .. }
                | FsOperation::Move { from, .. }
                | FsOperation::Copy { from, .. } => Some(from),
                FsOperation::Create { .. } => None,
            };
            if let Some(parent) = source.and_then(|path| path.parent()) {
                self.directory_tree.refresh(root.join(parent), show_hidden);
            }
            if let Some(target) = match operation {
                FsOperation::Create { path, .. } => Some(path),
                FsOperation::Rename { to, .. }
                | FsOperation::Move { to, .. }
                | FsOperation::Copy { to, .. } => Some(to),
                FsOperation::Delete { .. } => None,
            } {
                selected_destination = Some(root.join(target));
                if let Some(parent) = target.parent() {
                    self.directory_tree.refresh(root.join(parent), show_hidden);
                }
            }
        }
        if let Some(destination) = selected_destination {
            let _ = self.directory_tree.reveal(&destination, show_hidden);
        }
    }
}
