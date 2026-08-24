// SPDX-License-Identifier: MPL-2.0

//! Persistent-session discovery, selection, preview, and lifecycle requests.

// Application-module dependencies:
use super::{App, PathBuf, WorkspaceSwitchRequest};
#[cfg(unix)]
use super::{
    ListAction, ListPicker, PickerItem, WorkspaceEvent, WorkspaceServiceHandle,
    session_picker_preview,
};

impl App {
    #[cfg(unix)]
    pub fn attach_workspace_service(&mut self, service: WorkspaceServiceHandle) {
        self.ports.workspace_service = Some(service);
    }

    #[cfg(unix)]
    pub fn apply_workspace_event(&mut self, event: WorkspaceEvent) {
        match event {
            WorkspaceEvent::Refreshed { generation, result } => {
                if generation != self.workspace_generation {
                    return;
                }
                match result {
                    Ok(rows) => {
                        self.workspace_rows = rows;
                        // Self-correcting: a swap performed from another
                        // workspace can change this one's number without this
                        // host hearing about it, and every listing carries the
                        // catalog's current answer for every row.
                        if let Some(row) = self
                            .workspace_rows
                            .iter()
                            .find(|row| row.project_root == self.project_root)
                        {
                            let number = row.number;
                            self.note_workspace_number(number);
                        }
                        self.rebuild_workspace_picker();
                        self.request_selected_workspace_preview();
                    }
                    Err(error) => self.error(error),
                }
            }
            WorkspaceEvent::Previewed {
                generation,
                path,
                result,
            } => {
                if generation != self.workspace_preview_generation
                    || self.workspace_preview_target.as_ref() != Some(&path)
                {
                    return;
                }
                self.workspace_preview_target = None;
                self.workspace_previews.insert(path, result);
                if self
                    .list
                    .as_ref()
                    .is_some_and(|picker| picker.title.starts_with("Sessions"))
                {
                    self.rebuild_workspace_picker();
                }
            }
            WorkspaceEvent::Started {
                generation,
                path,
                result,
            } => {
                if generation != self.workspace_generation {
                    return;
                }
                match result {
                    Ok(()) => {
                        self.status(format!("started session for {}", path.display()));
                        self.request_workspace_refresh();
                    }
                    Err(error) => self.error(error),
                }
            }
            WorkspaceEvent::Stopped {
                generation,
                selector,
                result,
            } => {
                if generation != self.workspace_generation {
                    return;
                }
                self.session_action_menu = None;
                match result {
                    Ok(()) => {
                        self.status(format!("stopped session for {}", selector.display()));
                        self.request_workspace_refresh();
                    }
                    Err(error) => self.error(error),
                }
            }
            WorkspaceEvent::Forgotten {
                generation,
                path,
                result,
            } => {
                if generation != self.workspace_generation {
                    return;
                }
                self.session_action_menu = None;
                match result {
                    Ok(true) => {
                        self.status(format!("forgot session record for {}", path.display()));
                        self.request_workspace_refresh();
                    }
                    // The row is listed by something other than history, so
                    // nothing was removed and a refresh would still show it.
                    Ok(false) => self.status(format!(
                        "session for {} was not in the recent list",
                        path.display()
                    )),
                    Err(error) => self.error(error),
                }
            }
            WorkspaceEvent::Renamed {
                generation,
                path,
                name,
                result,
            } => {
                if generation != self.workspace_generation {
                    return;
                }
                self.session_action_menu = None;
                match result {
                    Ok(()) => {
                        self.status(format!("renamed session for {} to {name}", path.display()));
                        self.request_workspace_refresh();
                    }
                    Err(error) => self.error(error),
                }
            }
            WorkspaceEvent::Numbered {
                generation,
                path,
                number,
                result,
            } => {
                if generation != self.workspace_generation {
                    return;
                }
                self.session_action_menu = None;
                match result {
                    Ok(displaced) => {
                        // Numbering this workspace can change the one the
                        // status line shows, and the recents file is the only
                        // authority on it, so re-read rather than guessing
                        // from the request we sent.
                        self.refresh_workspace_number();
                        let subject = match number {
                            Some(number) => {
                                format!("session for {} is now {number}", path.display())
                            }
                            None => format!("session for {} has no number", path.display()),
                        };
                        match displaced {
                            Some(displaced) => self.status(format!(
                                "{subject}; {} took the number it gave up",
                                displaced.display()
                            )),
                            None => self.status(subject),
                        }
                        self.request_workspace_refresh();
                    }
                    Err(error) => self.error(error),
                }
            }
        }
    }

    pub(super) fn request_workspace_switch(&mut self, path: PathBuf) -> bool {
        self.request_workspace_switch_for_platform(path, cfg!(unix))
    }

    pub(super) fn request_workspace_switch_for_platform(
        &mut self,
        path: PathBuf,
        platform_supports_persistent_sessions: bool,
    ) -> bool {
        if self.reject_unsupported_persistent_session(platform_supports_persistent_sessions) {
            return false;
        }
        if !self.persistent_session {
            self.error("attaching sessions needs workspace.mode: persistent");
            return false;
        }
        self.workspace_switch = Some(WorkspaceSwitchRequest {
            selector: path,
            working_directory: self.working_directory.clone(),
        });
        true
    }

    #[cfg(unix)]
    pub(super) fn request_workspace_refresh(&mut self) {
        self.workspace_generation = self.workspace_generation.wrapping_add(1).max(1);
        let generation = self.workspace_generation;
        let Some(service) = self.ports.workspace_service.as_ref() else {
            self.error("session service is unavailable");
            return;
        };
        match service.try_refresh(generation) {
            Ok(()) => self.status("refreshing sessions…"),
            Err(error) => self.error(error),
        }
    }

    #[cfg(unix)]
    pub(super) fn rebuild_workspace_picker(&mut self) {
        let (filter, selected) = self
            .list
            .as_ref()
            .filter(|picker| picker.title.starts_with("Sessions"))
            .map_or_else(
                || (String::new(), 0),
                |picker| (picker.filter.clone(), picker.selected),
            );
        self.list_actions = (0..self.workspace_rows.len())
            .map(ListAction::Workspace)
            .collect();
        let items = self
            .workspace_rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let marker = if row.project_root == self.project_root {
                    "* "
                } else {
                    "  "
                };
                // Two display cells whether or not the row has a number, so
                // the names stay in one column and a numbered row is found by
                // where the digit is rather than by reading every line.
                let number = row
                    .number
                    .map_or_else(|| "  ".to_owned(), |number| format!("{number} "));
                let state = row.state_label();
                let mut details = vec![row.project_root.display().to_string(), state.to_owned()];
                // A successful health reply populates every live field,
                // including confirmed zeroes. Keep a timed-out reply distinct
                // from those omitted zeroes: this host may still own protected
                // state even though the bounded inspection did not answer.
                let health_available = row.unsaved_buffers.is_some()
                    && row.pending_wait_requests.is_some()
                    && row.live_terminals.is_some()
                    && row.terminal_sessions.is_some()
                    && row.interactive_attached.is_some();
                if row.running && row.incompatible_protocol.is_none() && !health_available {
                    details.push("health unavailable".to_owned());
                }
                // A confirmed count of zero is the uninteresting answer, and
                // the detail line is the only room a row has for anything
                // beyond its name. Dropping it leaves a quiet session reading
                // as its path and state, so a shown count is worth reading.
                if let Some(count) = row.unsaved_buffers.filter(|count| *count > 0) {
                    details.push(format!("unsaved {count}"));
                }
                if let Some(count) = row.live_terminals.filter(|count| *count > 0) {
                    details.push(format!("terminals {count}"));
                }
                if let Some(count) = row.pending_wait_requests.filter(|count| *count > 0) {
                    details.push(format!("waiting {count}"));
                }
                if let (Some(total), Some(live)) = (row.terminal_sessions, row.live_terminals) {
                    let exited = total.saturating_sub(live);
                    if exited > 0 {
                        details.push(format!("exited terminals {exited}"));
                    }
                }
                if let Some(attached) = row.interactive_attached {
                    details.push(if attached {
                        "TUI attached".to_owned()
                    } else {
                        "no TUI".to_owned()
                    });
                }
                PickerItem::new(
                    format!("{number}{marker}{}", row.display_name()),
                    details.join(" · "),
                    index,
                )
                .with_preview(session_picker_preview(
                    row,
                    self.workspace_previews.get(&row.project_root),
                    self.workspace_preview_target.as_ref() == Some(&row.project_root),
                ))
                // A stopped session is still worth listing and still starts
                // on Enter, so it stays in place rather than being hidden or
                // sorted away; dimming is what separates it from the hosts
                // that are actually up.
                .dimmed(!row.running)
            })
            .collect();
        let title = if self.persistent_session {
            "Sessions · 1-9 attach · Tab actions"
        } else {
            "Sessions · Enter cannot attach in standalone mode · Tab actions"
        };
        let mut picker = ListPicker::new(title, items).with_preview("Session");
        picker.primary_action = self.persistent_session.then(|| "attach".to_owned());
        picker.filter = filter;
        picker.selected = selected.min(self.workspace_rows.len().saturating_sub(1));
        self.list = Some(picker);
    }

    /// Starts one coalesced control request for the selected running session.
    /// Stopped and incompatible rows have complete static previews, while a
    /// successful live preview remains cached until the manager is reopened.
    #[cfg(unix)]
    pub(super) fn request_selected_workspace_preview(&mut self) {
        let Some(ListAction::Workspace(index)) = self.selected_list_action() else {
            return;
        };
        let Some(row) = self.workspace_rows.get(index) else {
            return;
        };
        if !row.running
            || row.incompatible_protocol.is_some()
            || self.workspace_previews.contains_key(&row.project_root)
            || self.workspace_preview_target.as_ref() == Some(&row.project_root)
        {
            return;
        }
        let path = row.project_root.clone();
        self.workspace_preview_generation =
            self.workspace_preview_generation.wrapping_add(1).max(1);
        let generation = self.workspace_preview_generation;
        self.workspace_preview_target = Some(path.clone());
        let result = self
            .ports
            .workspace_service
            .as_ref()
            .ok_or("session preview service is unavailable")
            .and_then(|service| service.try_preview(generation, path.clone()));
        if let Err(error) = result {
            self.workspace_preview_target = None;
            self.workspace_previews.insert(path, Err(error.to_owned()));
        }
        self.rebuild_workspace_picker();
    }

    #[cfg(unix)]
    pub(super) fn start_session(&mut self, workspace: PathBuf) {
        if !self.persistent_session {
            self.error("starting sessions needs workspace.mode: persistent");
            return;
        }
        self.workspace_generation = self.workspace_generation.wrapping_add(1).max(1);
        let generation = self.workspace_generation;
        let Some(service) = self.ports.workspace_service.as_ref() else {
            self.error("session service is unavailable");
            return;
        };
        match service.try_start(generation, workspace, self.working_directory.clone()) {
            Ok(()) => self.status("starting session…"),
            Err(error) => self.error(error),
        }
    }

    #[cfg(unix)]
    pub(super) fn stop_session(&mut self, selector: PathBuf) {
        self.stop_session_with_force(selector, false);
    }

    #[cfg(unix)]
    pub(super) fn stop_session_force(&mut self, selector: PathBuf) {
        self.stop_session_with_force(selector, true);
    }

    #[cfg(unix)]
    fn stop_session_with_force(&mut self, selector: PathBuf, force: bool) {
        if !self.persistent_session {
            self.error("stopping sessions needs workspace.mode: persistent");
            return;
        }
        self.workspace_generation = self.workspace_generation.wrapping_add(1).max(1);
        let generation = self.workspace_generation;
        let Some(service) = self.ports.workspace_service.as_ref() else {
            self.error("session service is unavailable");
            return;
        };
        match service.try_stop(generation, selector, self.working_directory.clone(), force) {
            Ok(()) => self.status(if force {
                "force-stopping session and its protected live state…"
            } else {
                "stopping session…"
            }),
            Err(error) => self.error(error),
        }
    }

    /// Drops a stopped session from the visited history behind the picker.
    #[cfg(unix)]
    pub(super) fn forget_workspace(&mut self, path: PathBuf) {
        self.workspace_generation = self.workspace_generation.wrapping_add(1).max(1);
        let generation = self.workspace_generation;
        let Some(service) = self.ports.workspace_service.as_ref() else {
            self.error("session service is unavailable");
            return;
        };
        match service.try_forget(generation, path) {
            Ok(()) => self.status("forgetting session record…"),
            Err(error) => self.error(error),
        }
    }

    #[cfg(unix)]
    pub(super) fn rename_session(&mut self, path: PathBuf, name: String) {
        self.workspace_generation = self.workspace_generation.wrapping_add(1).max(1);
        let generation = self.workspace_generation;
        let Some(service) = self.ports.workspace_service.as_ref() else {
            self.error("session service is unavailable");
            return;
        };
        match service.try_rename(generation, path, self.working_directory.clone(), name) {
            Ok(()) => self.status("renaming session…"),
            Err(error) => self.error(error),
        }
    }

    /// Asks the catalog to give this workspace a number, or to take its away.
    #[cfg(unix)]
    pub(super) fn number_session(&mut self, path: PathBuf, number: Option<u8>) {
        self.workspace_generation = self.workspace_generation.wrapping_add(1).max(1);
        let generation = self.workspace_generation;
        let Some(service) = self.ports.workspace_service.as_ref() else {
            self.error("session service is unavailable");
            return;
        };
        match service.try_number(generation, path, self.working_directory.clone(), number) {
            Ok(()) => self.status("numbering session…"),
            Err(error) => self.error(error),
        }
    }

    /// Records the number this workspace answers to, as the catalog reports it.
    ///
    /// Startup and every numbering change route through here so the status
    /// line has one source rather than inferring a number from whichever
    /// request it last sent.
    pub fn note_workspace_number(&mut self, number: Option<u8>) {
        self.workspace_number = number;
    }

    /// Re-reads this workspace's number from the per-user catalog.
    #[cfg(unix)]
    fn refresh_workspace_number(&mut self) {
        let number = crate::workspace::recorded_workspace_number(&self.project_root);
        self.note_workspace_number(number);
    }

    /// Enables the detach-and-preserve policy owned by a persistent session.
    pub fn enable_persistent_session(&mut self) {
        self.persistent_session = true;
    }
}
