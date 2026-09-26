// SPDX-License-Identifier: MPL-2.0

//! Navigation over the open working set; never starts a filesystem scan.

use super::{
    App, BufferAction, HashMap, ListAction, ListPicker, Mode, OpenDestination,
    OpenDestinationEntry, Path, PickerItem, Result, TerminalId,
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

    /// Which destination list is open, if any.
    ///
    /// The Navigator, the buffer list, and the terminal list are one list
    /// over three scopes; the title names the scope.
    pub(super) fn destination_scope(&self) -> Option<DestinationScope> {
        let title = &self.list.as_ref()?.title;
        [
            DestinationScope::All,
            DestinationScope::Buffers,
            DestinationScope::Terminals,
        ]
        .into_iter()
        .find(|scope| title.starts_with(scope.title_prefix()))
    }

    pub(super) fn navigator_open(&self) -> bool {
        self.destination_scope().is_some()
    }

    pub(super) fn open_navigator(&mut self) {
        self.open_destination_list(DestinationScope::All);
    }

    pub(super) fn open_destination_list(&mut self, scope: DestinationScope) {
        self.navigator_selection_lost = false;
        self.note_destination_activation();
        let rows = self.destination_rows(scope);
        let actions = rows
            .iter()
            .map(|row| ListAction::Destination(row.destination))
            .collect::<Vec<_>>();
        let order = (0..rows.len()).collect::<Vec<_>>();
        let (items, header) = self.destination_items(rows, &order);
        let context = self
            .project_root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        #[cfg(unix)]
        let context = self
            .workspace_rows
            .iter()
            .find(|row| row.project_root == self.project_root)
            .and_then(|row| row.name.clone())
            .unwrap_or(context);
        let picker = ListPicker::fuzzy(
            format!(
                "{}{context} · {}",
                scope.title_prefix(),
                scope.description()
            ),
            items,
        )
        .as_manager("visit", "Tab", "actions")
        .with_column_header(header.0, "", header.1)
        .with_preview(scope.preview_title())
        .with_key_legend();
        self.list = Some(picker);
        self.list_actions = actions;
    }

    /// The destinations `scope` lists, most recently activated first.
    ///
    /// The terminal list keeps the ones that can still be typed into ahead of
    /// the ones whose program has exited, which are history.
    fn destination_rows(&self, scope: DestinationScope) -> Vec<DestinationRow> {
        let mut rows = Vec::new();
        if scope != DestinationScope::Terminals {
            for (index, buffer) in self.buffers.iter().enumerate() {
                if self.buffer_is_discoverable(index) {
                    rows.push(self.buffer_destination_row(index, buffer));
                }
            }
        }
        if scope != DestinationScope::Buffers {
            for terminal in self.terminals.iter() {
                if terminal.live() || scope == DestinationScope::Terminals {
                    rows.push(self.terminal_destination_row(terminal));
                }
            }
        }
        rows.sort_by_key(|row| {
            (
                row.dimmed,
                std::cmp::Reverse(
                    self.destination_recency
                        .iter()
                        .position(|item| *item == row.destination),
                ),
            )
        });
        rows
    }

    fn destination_shown(&self, destination: OpenDestination) -> bool {
        self.panes
            .values()
            .any(|pane| pane.destination() == destination)
    }

    fn buffer_destination_row(&self, index: usize, buffer: &super::Buffer) -> DestinationRow {
        use crate::snapshot::RowTint;

        let (kind, tint) = match &buffer.kind {
            super::BufferKind::File => ("[file]".to_owned(), RowTint::File),
            super::BufferKind::Directory => ("[explorer]".to_owned(), RowTint::Explorer),
            super::BufferKind::Scratch => ("[scratch]".to_owned(), RowTint::Scratch),
            super::BufferKind::Provider(_) => ("[remote]".to_owned(), RowTint::File),
            _ => {
                let title = buffer.display_name();
                let kind = title
                    .split_once(']')
                    .map_or(title.clone(), |(kind, _)| format!("{kind}]"));
                (kind, RowTint::Generated)
            }
        };
        let name = match buffer.path.as_deref() {
            Some(path) => self.destination_path(path),
            None => {
                let title = buffer.display_name();
                let rest = title
                    .strip_prefix(kind.as_str())
                    .unwrap_or(&title)
                    .trim()
                    .to_owned();
                if rest.is_empty() {
                    kind.trim_start_matches('[')
                        .trim_end_matches(']')
                        .to_owned()
                } else {
                    rest
                }
            }
        };
        let mut flags = Vec::new();
        if buffer.dirty {
            flags.push(("[+]", RowTint::Modified));
        }
        if buffer.external_file_status().is_stale() {
            flags.push(("[STALE]", RowTint::Stale));
        }
        if buffer.is_read_only() {
            flags.push(("[RO]", RowTint::ReadOnly));
        }
        let mut fields = vec![buffer.display_name(), buffer.pane_title()];
        if let Some(path) = buffer.path.as_deref() {
            fields.extend(super::resource_path_fields(
                path,
                &self.project_root,
                self.home_directory.as_deref(),
            ));
        }
        let destination = OpenDestination::Buffer(index);
        DestinationRow {
            destination,
            kind,
            tint,
            name,
            flags,
            shown: self.destination_shown(destination),
            dimmed: false,
            fields,
            path: buffer.path.clone(),
            preview: super::buffer_preview(buffer),
        }
    }

    fn terminal_destination_row(&self, terminal: &super::TerminalSession) -> DestinationRow {
        use crate::snapshot::RowTint;

        let mut flags = Vec::new();
        if !terminal.live() {
            flags.push(("exited", RowTint::Exited));
        }
        if terminal.unread_activity() {
            flags.push(("unread", RowTint::Unread));
        }
        if terminal.bell() {
            flags.push(("bell", RowTint::Bell));
        }
        let directory = self.destination_path(terminal.directory());
        // The ID is how a command names a terminal, so the filter still
        // answers to it after the row stopped showing it.
        let mut fields = vec![
            terminal.name(),
            terminal.launch_label().to_owned(),
            terminal.directory().display().to_string(),
            directory.clone(),
            terminal.id().to_string(),
            format!("#{}", terminal.id()),
        ];
        fields.extend(terminal.child_title().map(str::to_owned));
        let mut preview = format!(
            "#{} · {} · {}\n\n",
            terminal.id(),
            terminal.launch_label(),
            directory
        );
        preview.push_str(&super::terminal_preview(terminal));
        let destination = OpenDestination::Terminal(terminal.id());
        DestinationRow {
            destination,
            kind: "[terminal]".to_owned(),
            tint: RowTint::Terminal,
            name: terminal.name(),
            flags,
            shown: self.destination_shown(destination),
            dimmed: !terminal.live(),
            fields,
            path: None,
            preview,
        }
    }

    /// A path as the NAME column writes it: relative inside the workspace,
    /// under `~` inside the home directory, and absolute otherwise.
    fn destination_path(&self, path: &Path) -> String {
        if let Ok(relative) = path.strip_prefix(&self.project_root) {
            return if relative.as_os_str().is_empty() {
                ".".to_owned()
            } else {
                relative.display().to_string()
            };
        }
        if let Some(home) = self.home_directory.as_deref()
            && let Ok(relative) = path.strip_prefix(home)
        {
            return if relative.as_os_str().is_empty() {
                "~".to_owned()
            } else {
                format!("~/{}", relative.display())
            };
        }
        path.display().to_string()
    }

    /// Lays `rows` out in aligned columns, each item indexing `order` for its
    /// action, and returns the column header with them.
    fn destination_items(
        &self,
        rows: Vec<DestinationRow>,
        order: &[usize],
    ) -> (Vec<PickerItem>, (String, String)) {
        use crate::snapshot::TintedRun;
        use unicode_width::UnicodeWidthStr as _;

        let pad = |text: &str, width: usize| {
            format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
        };
        let kind_width = rows
            .iter()
            .map(|row| row.kind.width())
            .max()
            .unwrap_or(0)
            .max("TYPE".width());
        let name_width = rows
            .iter()
            .map(|row| row.name.width())
            .max()
            .unwrap_or(0)
            .max("NAME".width());
        let states = rows
            .iter()
            .map(|row| {
                row.flags
                    .iter()
                    .map(|(flag, _)| *flag)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>();
        let state_width = states
            .iter()
            .map(|state| state.width())
            .max()
            .unwrap_or(0)
            .max("STATE".width());
        let items = rows
            .into_iter()
            .zip(states)
            .zip(order)
            .map(|((row, state), index)| {
                let prefix = format!(
                    "{} {}  ",
                    if row.shown { '*' } else { ' ' },
                    pad(&row.kind, kind_width),
                );
                let name_start = prefix.chars().count();
                let label = format!("{prefix}{}", pad(&row.name, name_width));
                let mut tints = vec![TintedRun {
                    trailing: false,
                    start: 2,
                    len: row.kind.chars().count(),
                    tint: row.tint,
                }];
                let mut start = 0;
                for (flag, tint) in &row.flags {
                    let len = flag.chars().count();
                    tints.push(TintedRun {
                        trailing: true,
                        start,
                        len,
                        tint: *tint,
                    });
                    start += len + 1;
                }
                let target = match row.destination {
                    OpenDestination::Buffer(buffer) => ResourceTarget::Buffer(buffer),
                    OpenDestination::Terminal(id) => ResourceTarget::Terminal(id),
                };
                let kind = match row.destination {
                    OpenDestination::Buffer(_) => ResourceKind::Buffer,
                    OpenDestination::Terminal(_) => ResourceKind::Terminal,
                };
                let mut resource = ResourceItem::new(label.clone(), "", target, kind, row.fields);
                resource.path = row.path;
                let mut item = PickerItem::new(label, "", *index)
                    .with_resource(resource)
                    .with_preview(row.preview)
                    .with_trailing_detail(pad(&state, state_width))
                    .dimmed(row.dimmed);
                item.tints = tints;
                item.elide_from = Some(name_start);
                item
            })
            .collect();
        (
            items,
            (
                format!("  {}  {}", pad("TYPE", kind_width), pad("NAME", name_width)),
                pad("STATE", state_width),
            ),
        )
    }

    /// Refresh metadata in the opening order with the original action indices.
    /// Removing a resource cannot make a stale selected row resolve to another
    /// resource.
    pub(super) fn refresh_navigator(&mut self) {
        let Some(scope) = self.destination_scope() else {
            return;
        };
        let selected = self
            .list
            .as_ref()
            .and_then(ListPicker::selected_item)
            .map(|item| item.index);
        let mut valid = self
            .destination_rows(scope)
            .into_iter()
            .map(|row| (row.destination, row))
            .collect::<HashMap<_, _>>();
        let mut rows = Vec::new();
        let mut order = Vec::new();
        for item in &self.list.as_ref().unwrap().items {
            let Some(ListAction::Destination(destination)) = self.list_actions.get(item.index)
            else {
                continue;
            };
            if let Some(row) = valid.remove(destination) {
                rows.push(row);
                order.push(item.index);
            }
        }
        let valid = rows
            .iter()
            .map(|row| row.destination)
            .collect::<std::collections::HashSet<_>>();
        let (items, header) = self.destination_items(rows, &order);
        let picker = self.list.as_mut().unwrap();
        picker.items = items;
        picker.column_header = Some(crate::picker::ListColumnHeader {
            label: header.0,
            detail: String::new(),
            trailing_detail: header.1,
        });
        // With no rows left there is nothing to have selected instead, so an
        // empty list still takes Tab.
        if !picker.items.is_empty()
            && selected
                .is_some_and(|selected| !picker.items.iter().any(|item| item.index == selected))
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
            .is_some_and(|menu| !valid.contains(&OpenDestination::Buffer(menu.buffer)))
        {
            self.buffer_action_menu = None;
        }
        // The menu an empty terminal list offers belongs to no terminal, so
        // only a menu about one terminal closes with its row.
        if self.terminal_action_menu.as_ref().is_some_and(|menu| {
            menu.actions.contains(&super::TerminalAction::Show)
                && !valid.contains(&OpenDestination::Terminal(menu.id))
        }) {
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
            // An exited terminal is still a screen to read; only the terminal
            // list offers one.
            OpenDestination::Terminal(id) => self.terminals.get(id).is_some(),
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
                if self.hidden_buffers_can_close() {
                    actions.push(BufferAction::CloseHidden);
                }
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
        for id in exited {
            self.close_terminal_id(id);
        }
        self.refresh_navigator();
        self.status(format!("closed {count} exited terminals"));
    }

    pub(super) fn attach_terminal_reported_directory(&mut self, id: TerminalId) {
        let directory = self
            .terminals
            .get(id)
            .and_then(|terminal| terminal.reported_directory())
            .map(std::path::Path::to_path_buf);
        if let Some(directory) = directory {
            if self.request_workspace_switch_for_platform(directory, cfg!(any(unix, windows))) {
                self.list = None;
                #[cfg(unix)]
                {
                    self.should_quit = true;
                }
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
            && self.request_workspace_switch_for_platform(path, cfg!(any(unix, windows)))
        {
            #[cfg(unix)]
            {
                self.should_quit = true;
            }
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

/// Which open destinations a destination list shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DestinationScope {
    /// The Navigator: every open buffer and running terminal.
    All,
    /// `Space b b`.
    Buffers,
    /// `Space t t`, including terminals whose program has exited.
    Terminals,
}

impl DestinationScope {
    const fn title_prefix(self) -> &'static str {
        match self {
            Self::All => "Navigator — ",
            Self::Buffers => "Buffers — ",
            Self::Terminals => "Terminals — ",
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::All => "Open buffers and terminals",
            Self::Buffers => "Open buffers",
            Self::Terminals => "Terminals",
        }
    }

    const fn preview_title(self) -> &'static str {
        match self {
            Self::All | Self::Buffers => "Contents",
            Self::Terminals => "Output",
        }
    }
}

/// One destination as its row says it, before the columns are aligned.
struct DestinationRow {
    destination: OpenDestination,
    kind: String,
    tint: crate::snapshot::RowTint,
    name: String,
    flags: Vec<(&'static str, crate::snapshot::RowTint)>,
    shown: bool,
    dimmed: bool,
    fields: Vec<String>,
    path: Option<std::path::PathBuf>,
    preview: String,
}
