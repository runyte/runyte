// SPDX-License-Identifier: MPL-2.0

use super::{
    App, ApplyReport, EditorCommand, EntryKind, FsOperation, Mode, PromptKind, TreePromptAction,
};
use crate::input_grammar::InputGrammar;
use crate::{
    input::{KeyCode, KeyStroke, Modifiers},
    layout::{Axis, Rect},
};
use anyhow::Result;

pub(crate) struct TreeDestination {
    pub path: std::path::PathBuf,
    pub panes: Vec<usize>,
    pub digits: String,
    pub split: Option<Axis>,
}

pub(super) struct TreeLegendCache {
    keymap: std::sync::Arc<crate::keymap::Keymap>,
    width: u16,
    lines: Vec<String>,
}

impl App {
    /// Handles sidebar commands before the backing pane's terminal or buffer
    /// can interpret them. The ordinary pane remains the file destination.
    pub(super) fn handle_directory_tree_command(&mut self, command: EditorCommand) -> Result<bool> {
        use EditorCommand as Command;
        let show_hidden = self
            .directory_tree
            .show_hidden(self.config.editor.show_hidden_files);
        if matches!(
            command,
            Command::DirectoryTreeNew
                | Command::DirectoryTreeRename
                | Command::DirectoryTreeMove
                | Command::DirectoryTreeDelete
        ) && !self.tree_filesystem_ready()
        {
            return Ok(true);
        }
        match command {
            Command::ToggleDirectoryTree => {
                if self.directory_tree.visible {
                    self.hide_directory_tree();
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
            Command::CloseWindow if self.directory_tree.focused => self.hide_directory_tree(),
            Command::GotoWord if self.directory_tree.focused => self.label_tree_entries(),
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
            Command::HalfPageUp if self.directory_tree.focused => self
                .directory_tree
                .select_relative(-((self.directory_tree.viewport_rows / 2).max(1) as isize)),
            Command::HalfPageDown if self.directory_tree.focused => self
                .directory_tree
                .select_relative((self.directory_tree.viewport_rows / 2).max(1) as isize),
            Command::GotoWindowTop | Command::GotoWindowCenter | Command::GotoWindowBottom
                if self.directory_tree.focused =>
            {
                let offset = match command {
                    Command::GotoWindowCenter => self.directory_tree.viewport_rows / 2,
                    Command::GotoWindowBottom => {
                        self.directory_tree.viewport_rows.saturating_sub(1)
                    }
                    _ => 0,
                };
                self.directory_tree
                    .select_row(self.directory_tree.scroll.saturating_add(offset));
            }
            Command::ToggleHiddenFiles if self.directory_tree.focused => {
                self.directory_tree
                    .toggle_hidden(self.config.editor.show_hidden_files);
            }
            Command::SearchRegex if self.directory_tree.focused => {
                self.open_prompt(PromptKind::DirectoryTreeSearch);
            }
            Command::SearchNext | Command::SearchPrevious if self.directory_tree.focused => {
                let result = self
                    .directory_tree
                    .search_next(command == Command::SearchNext);
                self.report_tree_search(result);
            }
            Command::DirectoryTreeOpen
            | Command::DirectoryTreeVertical
            | Command::DirectoryTreeHorizontal => {
                if self.directory_tree.kind(&self.directory_tree.selected)
                    == Some(EntryKind::Directory)
                {
                    self.directory_tree.toggle_selected(show_hidden);
                } else {
                    let split = match command {
                        Command::DirectoryTreeVertical => Some(Axis::Horizontal),
                        Command::DirectoryTreeHorizontal => Some(Axis::Vertical),
                        _ => None,
                    };
                    let panes = self.directory_tree_panes();
                    if panes.len() > 1 {
                        self.directory_tree_destination = Some(TreeDestination {
                            path: self.directory_tree.selected.clone(),
                            panes,
                            digits: String::new(),
                            split,
                        });
                        self.grammar.reset();
                    } else {
                        self.open_tree_destination(
                            self.directory_tree.selected.clone(),
                            self.active_pane,
                            split,
                        )?;
                    }
                }
            }
            Command::DirectoryTreeLegend => {
                self.directory_tree.legend_visible = !self.directory_tree.legend_visible
            }
            Command::DirectoryTreePane1 => self.open_tree_number(1)?,
            Command::DirectoryTreePane2 => self.open_tree_number(2)?,
            Command::DirectoryTreePane3 => self.open_tree_number(3)?,
            Command::DirectoryTreePane4 => self.open_tree_number(4)?,
            Command::DirectoryTreePane5 => self.open_tree_number(5)?,
            Command::DirectoryTreePane6 => self.open_tree_number(6)?,
            Command::DirectoryTreePane7 => self.open_tree_number(7)?,
            Command::DirectoryTreePane8 => self.open_tree_number(8)?,
            Command::DirectoryTreePane9 => self.open_tree_number(9)?,
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
                self.directory_tree.refresh(path);
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
                self.directory_tree_prompt_target = Some(parent);
                self.open_prompt(PromptKind::DirectoryTreeAction(TreePromptAction::New));
            }
            Command::DirectoryTreeRename | Command::DirectoryTreeMove => {
                let selected = self.directory_tree.selected.clone();
                if selected == self.directory_tree.root {
                    self.action_failed("the workspace root cannot be renamed or moved");
                    return Ok(true);
                }
                self.directory_tree_prompt_target = Some(selected.clone());
                if command == Command::DirectoryTreeRename {
                    let name = selected
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
                match self
                    .directory_tree
                    .prepare_delete(&self.directory_tree.selected, show_hidden)
                {
                    Ok(plan) => self.directory_tree_delete = Some(plan),
                    Err(error) => self.action_failed(error.to_string()),
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

    fn label_tree_entries(&mut self) {
        self.directory_tree_jump_paths.clear();
        let Some((area, _)) = self.directory_tree_geometry else {
            self.action_failed("no tree entries on screen to jump to");
            return;
        };
        let rows = self.directory_tree.rows();
        let selected = rows
            .iter()
            .position(|row| row.path == self.directory_tree.selected)
            .unwrap_or(0);
        let mut candidates = Vec::new();
        for (index, row) in rows
            .iter()
            .enumerate()
            .skip(self.directory_tree.scroll)
            .take(self.tree_body_height(area))
        {
            let indent = row.depth.saturating_mul(2).min(64);
            // Labels occupy the two marker cells before the name. Requiring
            // a visible name also excludes entries clipped by deep nesting.
            if indent + 2 >= usize::from(area.width.saturating_sub(2)) {
                continue;
            }
            let offset = self.directory_tree_jump_paths.len() * 2;
            self.directory_tree_jump_paths.push(row.path.clone());
            candidates.push((index.abs_diff(selected), index, offset));
        }
        candidates.sort_unstable();
        self.jump = crate::jump_labels::JumpLabels::new(
            candidates.into_iter().map(|(_, _, offset)| offset),
        );
        if let Some(labels) = &self.jump {
            self.status(format!("jump to tree entry: {} labels", labels.len()));
        } else {
            self.action_failed("no tree entries on screen to jump to");
        }
    }

    pub(super) fn report_tree_search(&mut self, result: Result<bool>) {
        match result {
            Ok(true) => self.status("tree match"),
            Ok(false) => self.action_failed("no matching tree entry"),
            Err(error) => self.action_failed(error.to_string()),
        }
    }

    fn tree_filesystem_ready(&mut self) -> bool {
        if self.fs_confirmation.is_some() || self.plugins.filesystem_confirmation.is_some() {
            self.action_warning(
                "Filesystem action blocked",
                "Finish the existing filesystem confirmation, then retry this tree action",
            );
            return false;
        }
        if self.plugins.filesystem_applying || !self.plugins.document_saves.is_empty() {
            self.action_warning(
                "Filesystem action waiting",
                "Wait for pending filesystem or document writes, then retry this tree action",
            );
            return false;
        }
        true
    }

    pub(super) fn enter_directory_tree(&mut self) {
        if !self.directory_tree.focused {
            self.directory_tree_previous_mode = self.mode;
            self.mode = Mode::Normal;
            self.directory_tree.focused = true;
            self.grammar.reset();
        }
    }

    pub(super) fn directory_tree_panes(&self) -> Vec<usize> {
        let mut panes = Vec::new();
        self.layout.panes(&mut panes);
        // Screen order, with layout order as the fallback before the first frame.
        panes.sort_by_key(|id| self.areas.get(id).map(|r| (r.y, r.x)).unwrap_or_default());
        panes
    }

    fn open_tree_number(&mut self, number: usize) -> Result<()> {
        if self.directory_tree.kind(&self.directory_tree.selected) == Some(EntryKind::Directory) {
            self.action_failed("select a file to open in a pane");
        } else if let Some(pane) = self.directory_tree_panes().get(number - 1).copied() {
            self.open_tree_destination(self.directory_tree.selected.clone(), pane, None)?;
        } else {
            self.action_failed("no pane with that number");
        }
        Ok(())
    }

    fn open_tree_destination(
        &mut self,
        path: std::path::PathBuf,
        pane: usize,
        split: Option<Axis>,
    ) -> Result<()> {
        if !self.panes.contains_key(&pane) {
            self.action_failed("the destination pane has closed");
            return Ok(());
        }
        self.activate_pane(pane);
        if let Some(axis) = split {
            self.split(axis, Some(path))?;
        } else {
            self.open_file(path)?;
        }
        self.directory_tree.focused = false;
        if self.prompt_kind != PromptKind::ExternalProgram {
            self.mode = Mode::Normal;
        }
        Ok(())
    }

    pub(super) fn handle_tree_destination_key(&mut self, key: KeyStroke) -> Result<()> {
        let Some(destination) = self.directory_tree_destination.as_mut() else {
            return Ok(());
        };
        let accept = match key.code {
            KeyCode::Escape => {
                self.directory_tree_destination = None;
                self.status("pane selection cancelled");
                return Ok(());
            }
            KeyCode::Backspace => {
                destination.digits.pop();
                false
            }
            KeyCode::Char(digit @ '0'..='9') if key.modifiers.is_empty() => {
                let candidate = format!("{}{digit}", destination.digits);
                if (1..=destination.panes.len()).any(|n| n.to_string().starts_with(&candidate)) {
                    destination.digits = candidate;
                }
                let matches = (1..=destination.panes.len())
                    .filter(|n| n.to_string().starts_with(&destination.digits))
                    .count();
                !destination.digits.is_empty() && matches == 1
            }
            KeyCode::Enter => !destination.digits.is_empty(),
            _ => false,
        };
        if accept {
            let number = destination.digits.parse::<usize>().unwrap_or_default();
            if let Some(pane) = number
                .checked_sub(1)
                .and_then(|n| destination.panes.get(n))
                .copied()
            {
                let destination = self.directory_tree_destination.take().unwrap();
                self.open_tree_destination(destination.path, pane, destination.split)?;
            }
        }
        Ok(())
    }

    pub(super) fn handle_tree_delete_key(&mut self, key: KeyStroke) {
        if key
            .modifiers
            .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
        {
            return;
        }
        match key.code {
            KeyCode::Char('y' | 'Y') => {
                if self.tree_filesystem_ready()
                    && let Some(plan) = self.directory_tree_delete.take()
                {
                    self.apply_native_filesystem_plan(plan, None, super::DeletionMode::Trash);
                }
            }
            KeyCode::Char('n' | 'N') | KeyCode::Enter | KeyCode::Escape => {
                self.directory_tree_delete = None;
                self.status("deletion cancelled");
            }
            _ => {}
        }
    }

    pub(crate) fn tree_interaction(&self) -> Option<String> {
        if let Some(delete) = &self.directory_tree_delete {
            let path = match delete.operations().first()? {
                FsOperation::Delete { path, .. } => path,
                _ => return None,
            };
            return Some(format!("Delete {}? [y/N]", path.display()));
        }
        self.directory_tree_destination.as_ref().map(|destination| {
            format!(
                "{} pane (1–{}, Escape cancels): {}",
                if destination.split.is_some() {
                    "Split"
                } else {
                    "Open in"
                },
                destination.panes.len(),
                destination.digits,
            )
        })
    }

    pub(super) fn tree_width(&self, editor_width: u16) -> u16 {
        let configured = self.config.editor.directory_tree_width;
        let preferred = self
            .directory_tree
            .width_override
            .filter(|(baseline, _)| *baseline == configured)
            .map(|(_, width)| width)
            .unwrap_or(configured.clamp(12, 240) as u16);
        preferred.clamp(12, editor_width.saturating_sub(24).max(12))
    }

    pub(super) fn resize_tree(&mut self, width: i32, editor_width: u16) {
        let width = width.clamp(12, i32::from(editor_width.saturating_sub(24).max(12))) as u16;
        self.directory_tree.width_override = Some((self.config.editor.directory_tree_width, width));
    }

    pub(crate) fn tree_legend(&self, area: Rect) -> Vec<String> {
        if !self.directory_tree.legend_visible || area.height < 5 {
            return Vec::new();
        }
        let mut cache = self.directory_tree_legend.borrow_mut();
        if let Some(cached) = cache.as_ref()
            && std::sync::Arc::ptr_eq(&cached.keymap, &self.keymap)
            && cached.width == area.width
        {
            return cached
                .lines
                .iter()
                .take(usize::from(area.height.saturating_sub(4)))
                .cloned()
                .collect();
        }
        use crate::keymap::{BindingScope, BindingTarget};
        use EditorCommand as Command;
        let actions = [
            (Command::DirectoryTreeNew, "new"),
            (Command::DirectoryTreeDelete, "delete"),
            (Command::DirectoryTreeMove, "move"),
            (Command::DirectoryTreeRename, "rename"),
            (Command::DirectoryTreeVertical, "open in v-split"),
            (Command::DirectoryTreeHorizontal, "open in h-split"),
            (Command::DirectoryTreeLegend, "legend"),
            (Command::ToggleHiddenFiles, "hidden files"),
            (Command::SearchRegex, "search"),
        ];
        let bindings = self
            .keymap
            .bindings_for_scope(Mode::Normal, BindingScope::DirectoryTree)
            .collect::<Vec<_>>();
        let text = actions
            .into_iter()
            .filter_map(|(command, label)| {
                bindings
                    .iter()
                    .find(|binding| binding.target == BindingTarget::Editor(command))
                    .map(|binding| format!("{}: {label}", binding.sequence))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let width = usize::from(area.width.saturating_sub(2)).max(1);
        let mut lines = vec![String::new()];
        for word in text.split_whitespace() {
            let line = lines.last_mut().unwrap();
            if !line.is_empty()
                && unicode_width::UnicodeWidthStr::width(line.as_str())
                    + 1
                    + unicode_width::UnicodeWidthStr::width(word)
                    > width
            {
                lines.push(word.to_owned());
            } else {
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
            }
        }
        *cache = Some(TreeLegendCache {
            keymap: self.keymap.clone(),
            width: area.width,
            lines: lines.clone(),
        });
        lines.truncate(usize::from(area.height.saturating_sub(4)));
        lines
    }

    pub(crate) fn tree_body_height(&self, area: Rect) -> usize {
        let legend = self.tree_legend(area);
        usize::from(area.height.saturating_sub(2))
            .saturating_sub(if legend.is_empty() {
                0
            } else {
                legend.len() + 1
            })
            .min(crate::snapshot::MAX_DIRECTORY_TREE_SNAPSHOT_ROWS)
    }

    pub(super) fn hide_directory_tree(&mut self) {
        self.directory_tree.visible = false;
        self.leave_directory_tree();
    }

    pub(super) fn leave_directory_tree(&mut self) {
        if self.directory_tree.focused {
            self.jump = None;
            self.directory_tree_jump_paths.clear();
            self.directory_tree.focused = false;
            if self.mode != Mode::Command {
                self.mode = self.directory_tree_previous_mode;
            }
        }
    }

    pub(super) fn accept_directory_tree_prompt(
        &mut self,
        action: TreePromptAction,
        target: std::path::PathBuf,
        value: &str,
    ) {
        if !self.tree_filesystem_ready() {
            // Submission can race an asynchronous save. Keep the entered
            // value so Enter can retry once the writer has finished.
            self.directory_tree_prompt_target = Some(target);
            self.open_prompt_with_value(PromptKind::DirectoryTreeAction(action), value.to_owned());
            return;
        }
        let show_hidden = self
            .directory_tree
            .show_hidden(self.config.editor.show_hidden_files);
        let result = match action {
            TreePromptAction::New => {
                self.directory_tree
                    .prepare_create(&target, value, show_hidden)
            }
            TreePromptAction::Rename => {
                self.directory_tree
                    .prepare_rename(&target, value, show_hidden)
            }
            TreePromptAction::Move => self
                .directory_tree
                .prepare_move(&target, value, show_hidden),
        };
        match result {
            Ok(plan) => self.apply_native_filesystem_plan(plan, None, super::DeletionMode::Trash),
            Err(error) => self.action_failed(error.to_string()),
        }
    }

    pub(super) fn refresh_directory_tree_after_report(
        &mut self,
        root: &std::path::Path,
        report: &ApplyReport,
    ) {
        self.directory_tree.note_applied(root, report);
        let show_hidden = self
            .directory_tree
            .show_hidden(self.config.editor.show_hidden_files);
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
                self.directory_tree
                    .refresh(super::resolved_operation_path(root, parent));
            }
            if let Some(target) = match operation {
                FsOperation::Create { path, .. } => Some(path),
                FsOperation::Rename { to, .. }
                | FsOperation::Move { to, .. }
                | FsOperation::Copy { to, .. } => Some(to),
                FsOperation::Delete { .. } => None,
            } {
                selected_destination = Some(super::resolved_operation_path(root, target));
                if let Some(parent) = target.parent() {
                    self.directory_tree
                        .refresh(super::resolved_operation_path(root, parent));
                }
            }
        }
        if let Some(destination) = selected_destination {
            let _ = self.directory_tree.reveal(&destination, show_hidden);
            // The destination may have been collapsed and uncached before
            // note_applied published its new entry. Read its siblings now
            // that reveal has expanded the parent.
            if let Some(parent) = destination.parent() {
                self.directory_tree.refresh(parent.to_path_buf());
            }
        }
    }
}
