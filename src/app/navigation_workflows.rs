// SPDX-License-Identifier: MPL-2.0

//! Navigation over the open working set; never starts a filesystem scan.

use super::{
    App, BufferAction, HashMap, ListAction, ListPicker, Mode, OpenDestination,
    OpenDestinationEntry, PickerItem, Result, TerminalId,
};
use crate::finder::{ResourceItem, ResourceKind, ResourceTarget};

impl App {
    pub fn open_destination_inventory(&self) -> Vec<OpenDestinationEntry> {
        let mut entries = Vec::new();
        for (index, buffer) in self.buffers.iter().enumerate() {
            if !self.buffer_is_discoverable(index) {
                continue;
            }
            let mut label = match &buffer.kind {
                super::BufferKind::File | super::BufferKind::Directory => {
                    let kind = if buffer.is_directory() {
                        "explorer"
                    } else {
                        "file"
                    };
                    let path = buffer
                        .path
                        .as_deref()
                        .map(|path| {
                            path.strip_prefix(&self.project_root)
                                .unwrap_or(path)
                                .display()
                                .to_string()
                        })
                        .unwrap_or_default();
                    format!("[{kind}] {}", if path.is_empty() { "." } else { &path })
                }
                _ => buffer.pane_title(),
            };
            if buffer.dirty {
                label.push_str(" [+]");
            }
            if buffer.external_file_status().is_stale() {
                label.push_str(" [STALE]");
            }
            if buffer.is_read_only() {
                label.push_str(" [RO]");
            }
            let detail = String::new();
            entries.push(OpenDestinationEntry {
                destination: OpenDestination::Buffer(index),
                label,
                detail,
            });
        }
        for terminal in self.terminals.iter().filter(|terminal| terminal.live()) {
            let mut detail = "running".to_owned();
            if let Some(title) = terminal
                .child_title()
                .filter(|title| *title != terminal.name())
            {
                detail.push_str(&format!(" · {title}"));
            }
            if terminal.launch_label() != terminal.name() {
                detail.push_str(&format!(" · {}", terminal.launch_label()));
            }
            entries.push(OpenDestinationEntry {
                destination: OpenDestination::Terminal(terminal.id()),
                label: terminal.display_name(),
                detail,
            });
        }
        for entry in &mut entries {
            let mut panes = self
                .panes
                .iter()
                .filter(|(_, pane)| pane.destination() == entry.destination)
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            panes.sort_unstable();
            if !panes.is_empty() {
                let labels = panes
                    .iter()
                    .map(|id| (id + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                entry.detail.push_str(&format!(" · pane {labels}"));
            }
        }
        entries
    }

    pub(super) fn note_destination_activation(&mut self) {
        if let Some(worker) = &self.syntax_worker {
            worker.prioritize(self.active().buffer);
        }
        let current = self.active().destination();
        self.destination_recency
            .retain(|destination| *destination != current);
        self.destination_recency.push(current);
    }

    pub(super) fn navigator_open(&self) -> bool {
        self.list
            .as_ref()
            .is_some_and(|list| list.title.starts_with("Navigator —"))
    }

    pub(super) fn open_navigator(&mut self) {
        self.navigator_selection_lost = false;
        self.note_destination_activation();
        let mut entries = self.open_destination_inventory();
        entries.sort_by_key(|entry| {
            std::cmp::Reverse(
                self.destination_recency
                    .iter()
                    .position(|item| *item == entry.destination),
            )
        });
        let items = entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| self.navigator_item(entry, index))
            .collect();
        let mut context = self
            .project_root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        #[cfg(unix)]
        if let Some(name) = self
            .workspace_rows
            .iter()
            .find(|row| row.project_root == self.project_root)
            .and_then(|row| row.name.clone())
        {
            context = name;
        }
        let mut picker = ListPicker::fuzzy(
            format!("Navigator — {context} · Open buffers and terminals"),
            items,
        )
        .as_manager("visit", "Tab", "actions");
        picker.show_preview = false;
        self.list = Some(picker);
        self.list_actions = self
            .list
            .as_ref()
            .unwrap()
            .items
            .iter()
            .filter_map(|item| {
                item.resource().map(|resource| match resource.target {
                    ResourceTarget::Buffer(buffer) => {
                        ListAction::Destination(OpenDestination::Buffer(buffer))
                    }
                    ResourceTarget::Terminal(terminal) => {
                        ListAction::Destination(OpenDestination::Terminal(terminal))
                    }
                    _ => unreachable!("Navigator has no content targets"),
                })
            })
            .collect();
    }

    fn navigator_item(&self, entry: OpenDestinationEntry, index: usize) -> PickerItem {
        let resource = match entry.destination {
            OpenDestination::Buffer(buffer) => {
                let state = &self.buffers[buffer];
                let mut fields = vec![state.display_name()];
                if let Some(path) = &state.path {
                    fields.push(path.to_string_lossy().into_owned());
                }
                let mut resource = ResourceItem::new(
                    entry.label.clone(),
                    "",
                    ResourceTarget::Buffer(buffer),
                    ResourceKind::Buffer,
                    fields,
                );
                resource.path = state.path.clone();
                resource
            }
            OpenDestination::Terminal(id) => {
                let terminal = self.terminals.get(id).unwrap();
                ResourceItem::new(
                    entry.label.clone(),
                    "",
                    ResourceTarget::Terminal(id),
                    ResourceKind::Terminal,
                    [
                        terminal.name(),
                        terminal.launch_label().to_owned(),
                        terminal.child_title().unwrap_or_default().to_owned(),
                    ],
                )
            }
        };
        PickerItem::new(entry.label, entry.detail, index).with_resource(resource)
    }

    /// Refresh metadata in the opening order with the original action indices.
    /// Removing a resource cannot make a stale selected row resolve to another
    /// resource.
    pub(super) fn refresh_navigator(&mut self) {
        if !self.navigator_open() {
            return;
        }
        let selected = self
            .list
            .as_ref()
            .and_then(ListPicker::selected_item)
            .map(|item| item.index);
        let valid = self
            .open_destination_inventory()
            .into_iter()
            .map(|entry| (entry.destination, entry))
            .collect::<HashMap<_, _>>();
        let items = self
            .list
            .as_ref()
            .unwrap()
            .items
            .iter()
            .filter_map(|item| {
                let ListAction::Destination(destination) = self.list_actions.get(item.index)?
                else {
                    return None;
                };
                let entry = valid.get(destination)?;
                Some(self.navigator_item(entry.clone(), item.index))
            })
            .collect();
        let picker = self.list.as_mut().unwrap();
        picker.items = items;
        if selected.is_some_and(|selected| !picker.items.iter().any(|item| item.index == selected))
        {
            self.navigator_selection_lost = true;
        }
        picker.selected = selected
            .and_then(|selected| {
                picker
                    .visible_indices()
                    .iter()
                    .position(|index| picker.items[*index].index == selected)
            })
            .unwrap_or(0);
        if self
            .buffer_action_menu
            .as_ref()
            .is_some_and(|menu| !valid.contains_key(&OpenDestination::Buffer(menu.buffer)))
        {
            self.buffer_action_menu = None;
        }
        if self
            .terminal_action_menu
            .as_ref()
            .is_some_and(|menu| !valid.contains_key(&OpenDestination::Terminal(menu.id)))
        {
            self.terminal_action_menu = None;
        }
    }

    pub fn visit_open_destination(&mut self, destination: OpenDestination) -> bool {
        self.visit_destination(destination, false)
    }

    pub(super) fn visit_destination(
        &mut self,
        destination: OpenDestination,
        bring_here: bool,
    ) -> bool {
        let valid = match destination {
            OpenDestination::Buffer(buffer) => {
                buffer < self.buffers.len() && self.buffer_is_discoverable(buffer)
            }
            OpenDestination::Terminal(id) => self
                .terminals
                .get(id)
                .is_some_and(|terminal| terminal.live()),
        };
        if !valid {
            self.action_failed("that destination is no longer open");
            return false;
        }
        if !bring_here {
            let visible = self
                .panes
                .iter()
                .filter(|(_, pane)| pane.destination() == destination)
                .map(|(id, _)| *id)
                .max_by_key(|id| (*id == self.active_pane, self.pane_focus_rank(*id)));
            if let Some(pane) = visible {
                self.activate_pane(pane);
            }
        }
        match destination {
            OpenDestination::Buffer(buffer) => {
                self.switch_buffer(buffer);
                self.mode = Mode::Normal;
            }
            OpenDestination::Terminal(id) => self.show_terminal(id),
        }
        self.note_destination_activation();
        true
    }

    pub(super) fn previous_destination(&mut self) {
        let current = self.active().destination();
        let target = self
            .active()
            .destination_history
            .iter()
            .rev()
            .copied()
            .find(|destination| {
                *destination != current
                    && match destination {
                        OpenDestination::Buffer(buffer) => self.buffer_is_discoverable(*buffer),
                        OpenDestination::Terminal(id) => self
                            .terminals
                            .get(*id)
                            .is_some_and(|terminal| terminal.live()),
                    }
            });
        if let Some(target) = target {
            self.visit_destination(target, true);
        } else {
            self.status("no previous open destination in this pane");
        }
    }

    pub(super) fn open_navigator_actions(&mut self, destination: OpenDestination) {
        match destination {
            OpenDestination::Buffer(buffer) => {
                let mut actions = self.available_buffer_actions(buffer);
                actions.insert(0, BufferAction::BringHere);
                self.buffer_action_menu = Some(super::BufferActionMenu {
                    buffer,
                    actions,
                    selected: 0,
                });
            }
            OpenDestination::Terminal(id) => self.open_terminal_actions_for(id),
        }
    }

    pub(super) fn close_exited_terminals(&mut self) {
        let exited = self
            .terminals
            .iter()
            .filter(|terminal| !terminal.live())
            .map(|terminal| terminal.id())
            .collect::<Vec<_>>();
        if exited.is_empty() {
            self.status("no exited terminals to close");
            return;
        }
        let count = exited.len();
        let filter = self
            .list
            .as_ref()
            .map(|list| list.filter.clone())
            .unwrap_or_default();
        let selected = self.list.as_ref().map_or(0, |list| list.selected);
        for id in exited {
            self.close_terminal_id(id);
        }
        self.rebuild_terminal_list();
        if let Some(list) = &mut self.list {
            list.filter = filter;
            list.selected = selected.min(list.visible_indices().len().saturating_sub(1));
        }
        self.status(format!("closed {count} exited terminals"));
    }

    pub(super) fn attach_terminal_reported_directory(&mut self, id: TerminalId) {
        let directory = self
            .terminals
            .get(id)
            .and_then(|terminal| terminal.reported_directory())
            .map(std::path::Path::to_path_buf);
        if let Some(directory) = directory {
            if self.request_workspace_switch(directory) {
                self.list = None;
                self.should_quit = true;
            }
        } else {
            self.action_failed("terminal has not reported a validated directory (OSC 7)");
        }
    }

    pub(super) fn open_explorer_session(&mut self) {
        if !self.buffers[self.active().buffer].is_directory() {
            self.action_failed("active buffer is not an explorer");
            return;
        }
        if let Some(path) = self.buffers[self.active().buffer].path.clone()
            && self.request_workspace_switch(path)
        {
            self.should_quit = true;
        }
    }

    pub fn set_parent_wait_buffers(&mut self, buffers: HashMap<usize, usize>) {
        self.parent_wait_origins
            .retain(|buffer, _| buffers.contains_key(buffer));
        self.parent_wait_buffers = buffers;
    }

    pub fn take_parent_wait_actions(&mut self) -> Vec<(usize, bool)> {
        std::mem::take(&mut self.parent_wait_actions)
    }

    pub fn parent_wait_origin_available(&self, terminal: TerminalId) -> bool {
        self.terminals
            .get(terminal)
            .is_some_and(|terminal| terminal.live())
            && self.panes.values().any(|pane| {
                pane.terminal == Some(terminal)
                    || pane.covered_terminal.is_some_and(|(_, id)| id == terminal)
            })
    }

    pub fn begin_parent_wait(&mut self, terminal: TerminalId, buffers: &[usize]) -> Result<()> {
        let pane = self
            .panes
            .iter()
            .find(|(_, pane)| {
                pane.terminal == Some(terminal)
                    || pane.covered_terminal.is_some_and(|(_, id)| id == terminal)
            })
            .map(|(id, _)| *id)
            .ok_or_else(|| anyhow::anyhow!("originating terminal is no longer visible"))?;
        anyhow::ensure!(
            self.terminals
                .get(terminal)
                .is_some_and(|terminal| terminal.live()),
            "originating terminal has exited"
        );
        for buffer in buffers {
            self.parent_wait_origins.insert(*buffer, (pane, terminal));
        }
        self.activate_pane(pane);
        if let Some(buffer) = buffers.first().copied() {
            self.switch_buffer(buffer);
            self.active_mut().covered_terminal = Some((buffer, terminal));
            self.mode = Mode::Normal;
            self.status(":wq saves and returns to terminal");
        }
        Ok(())
    }

    pub(super) fn complete_parent_wait(&mut self, cancel: bool) -> bool {
        let buffer = self.active().buffer;
        if self.document_mutation_pending(buffer) {
            self.action_warning(
                "Save pending",
                "Wait for the document write before returning",
            );
            return true;
        }
        if self.active_terminal().is_some() || !self.parent_wait_buffers.contains_key(&buffer) {
            return false;
        }
        if self.buffers[buffer].dirty {
            if !cancel {
                self.action_failed("unsaved changes · :wq saves and returns to terminal");
                return true;
            }
            if self.parent_wait_buffers[&buffer] > 1 {
                self.action_failed(
                    "another pending request shares this buffer; save before canceling",
                );
                return true;
            }
            if let Err(error) = self.discard_buffer_changes(buffer) {
                self.action_failed(error.to_string());
                return true;
            }
        }
        self.parent_wait_actions.push((buffer, cancel));
        self.parent_wait_buffers.remove(&buffer);
        if let Some((pane_id, terminal)) = self.parent_wait_origins.remove(&buffer)
            && self
                .panes
                .get(&pane_id)
                .is_some_and(|pane| pane.terminal.is_none() && pane.buffer == buffer)
            && self
                .terminals
                .get(terminal)
                .is_some_and(|terminal| terminal.live())
            && !self
                .panes
                .iter()
                .any(|(id, pane)| *id != pane_id && pane.terminal == Some(terminal))
        {
            self.activate_pane(pane_id);
            self.show_terminal(terminal);
        }
        self.status(if cancel {
            "external editor request canceled"
        } else {
            "external edit completed"
        });
        true
    }
}
