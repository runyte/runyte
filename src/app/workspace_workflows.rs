// SPDX-License-Identifier: MPL-2.0

//! Persistent-session discovery, selection, preview, and lifecycle requests.

// Application-module dependencies:
use super::{App, PathBuf, WorkspaceSwitchRequest};
#[cfg(unix)]
use super::{
    ListAction, ListPicker, PickerItem, WorkspaceEvent, WorkspaceServiceHandle,
    compact_session_elapsed, session_picker_preview,
};

impl App {
    #[cfg(unix)]
    pub fn attach_workspace_service(&mut self, service: WorkspaceServiceHandle) {
        self.ports.workspace_service = Some(service);
    }

    #[cfg(unix)]
    pub fn apply_workspace_event(&mut self, event: WorkspaceEvent) {
        match event {
            WorkspaceEvent::Inspected {
                generation,
                path,
                result,
            } => self.finish_worktree_session_check(generation, path, result),
            WorkspaceEvent::Refreshed { generation, result } => {
                if generation != self.workspace_generation {
                    return;
                }
                match result {
                    Ok(rows) => {
                        let manager_open = self
                            .list
                            .as_ref()
                            .is_some_and(|picker| picker.title.starts_with("Sessions"));
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
                        if manager_open {
                            self.rebuild_workspace_picker();
                            self.request_selected_workspace_preview();
                        }
                    }
                    Err(error) => self.error_from("Host", "Host operation failed", error),
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
                    Err(error) => self.error_from("Host", "Host operation failed", error),
                }
            }
            WorkspaceEvent::Stopped {
                generation,
                selector,
                result,
            } => {
                // A stop that belongs to a compound worktree removal is one
                // step of it, not an answer: reporting "stopped session" and
                // stopping there would leave the worktree standing. Match its
                // own request before the manager's latest-generation gate: a
                // refresh requested while the host is stopping must not make
                // this reply disappear.
                if self
                    .worktree_teardown
                    .as_ref()
                    .is_some_and(|teardown| teardown.awaits_stop(generation, &selector))
                {
                    self.session_action_menu = None;
                    match result {
                        Ok(()) => {
                            self.advance_worktree_teardown(super::WorktreeTeardownStage::Removing);
                            self.continue_worktree_teardown_after_stop();
                        }
                        Err(error) => {
                            self.worktree_teardown = None;
                            self.error_from(
                                "Host",
                                "Host operation failed",
                                format!(
                                    "cannot remove this worktree because its session could not be stopped: {error}"
                                ),
                            );
                        }
                    }
                    return;
                }
                if generation != self.workspace_generation {
                    return;
                }
                self.session_action_menu = None;
                match result {
                    Ok(()) => {
                        self.status(format!("stopped session for {}", selector.display()));
                        self.request_workspace_refresh();
                    }
                    Err(error) => self.error_from("Host", "Host operation failed", error),
                }
            }
            WorkspaceEvent::Forgotten {
                generation,
                path,
                result,
            } => {
                // The record is the last level of a compound removal. Whether
                // one was there to remove says nothing about whether the
                // worktree went, which has already happened by now, so the
                // cascade reports what it did either way. As with its stop,
                // this request remains authoritative if an unrelated manager
                // refresh has since advanced the shared generation.
                if self
                    .worktree_teardown
                    .as_ref()
                    .is_some_and(|teardown| teardown.awaits_forget(generation, &path))
                {
                    self.session_action_menu = None;
                    // Queue the projection refresh before producing the
                    // compound action's final answer. Refresh submission has
                    // its own progress message; doing it afterwards would
                    // overwrite both a successful teardown summary and a
                    // failure to forget the now-removed workspace's record.
                    self.request_workspace_refresh();
                    if let Err(error) = result {
                        // The directory is already gone; a stranded record is
                        // worth saying out loud, because it is what keeps a
                        // number claimed. A branch above it is left intact:
                        // failure at this level stops the cascade here.
                        self.fail_worktree_teardown_after_removal(error);
                    } else {
                        self.finish_worktree_teardown();
                    }
                    return;
                }
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
                    Err(error) => self.error_from("Host", "Host operation failed", error),
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
                    Err(error) => self.error_from("Host", "Host operation failed", error),
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
                    Err(error) => self.error_from("Host", "Host operation failed", error),
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
        if self.reject_unavailable_persistent_session(platform_supports_persistent_sessions, false)
        {
            return false;
        }
        if !self.persistent_session {
            self.action_failed("attaching sessions needs workspace.mode: persistent");
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
            self.action_failed("session service is unavailable");
            return;
        };
        match service.try_refresh(generation) {
            Ok(()) => self.status("refreshing sessions…"),
            Err(error) => self.error_from("Host", "Host operation failed", error),
        }
    }

    #[cfg(unix)]
    pub(super) fn rebuild_workspace_picker(&mut self) {
        self.rebuild_workspace_picker_at(session_activity_now());
    }

    #[cfg(unix)]
    fn rebuild_workspace_picker_at(&mut self, now: u64) {
        use unicode_width::UnicodeWidthStr as _;

        let (filter, selected) = self
            .list
            .as_ref()
            .filter(|picker| picker.title.starts_with("Sessions"))
            .map_or_else(
                || (String::new(), 0),
                |picker| {
                    let selected = if picker.title == "Sessions · loading…" {
                        self.workspace_rows
                            .iter()
                            .position(|row| row.project_root == self.project_root)
                            .unwrap_or(0)
                    } else {
                        picker.selected
                    };
                    (picker.filter.clone(), selected)
                },
            );
        self.list_actions = (0..self.workspace_rows.len())
            .map(ListAction::Workspace)
            .collect();
        // The manager reads as five columns — number, name, branch, directory,
        // activity —
        // padded to the widest value in the list so they line up down it, the
        // way the contextual action menu already lines its own columns up. A
        // row that is not in a Git repository still pays for its branch column
        // and says `-` there. Its directory is always useful session identity,
        // whether or not Git considers that directory a linked worktree.
        let columns = self
            .workspace_rows
            .iter()
            .map(|row| {
                let last_active = if row.project_root == self.project_root {
                    Some(now)
                } else {
                    row.last_active_unix_seconds
                };
                (
                    row.display_name(),
                    Self::session_branch_cell(row),
                    self.session_directory_cell(row),
                    compact_session_elapsed(last_active, now),
                )
            })
            .collect::<Vec<_>>();
        let name_width = columns
            .iter()
            .map(|(name, _, _, _)| name.width())
            .max()
            .unwrap_or(0)
            .max("Name".width());
        let branch_width = columns
            .iter()
            .map(|(_, branch, _, _)| branch.width())
            .max()
            .unwrap_or(0)
            .max("Branch".width());
        let directory_width = columns
            .iter()
            .map(|(_, _, directory, _)| directory.width())
            .max()
            .unwrap_or(0)
            .max("Path".width());
        let items = self
            .workspace_rows
            .iter()
            .zip(columns.iter())
            .enumerate()
            .map(|(index, (row, (name, branch, directory, active)))| {
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
                let label = format!(
                    "{number}{marker}{name}{}",
                    " ".repeat(name_width.saturating_sub(name.width()))
                );
                let detail = format!(
                    "{branch}{}  {directory}{}",
                    " ".repeat(branch_width.saturating_sub(branch.width())),
                    " ".repeat(directory_width.saturating_sub(directory.width()))
                );
                // The padding is presentation, so it is kept out of the
                // haystack: filtering answers to what the row says, not to how
                // wide the widest other row happened to be.
                // Keep the absolute identity searchable even when the visible
                // directory is shortened through the configured home path.
                let search = format!(
                    "{name} {} {branch} {directory}",
                    crate::git::display_path(&row.project_root)
                );
                PickerItem::searchable(label, detail, search, index)
                    .with_trailing_detail(active)
                    .with_preview(session_picker_preview(
                        row,
                        self.workspace_previews.get(&row.project_root),
                        self.workspace_preview_target.as_ref() == Some(&row.project_root),
                        active,
                    ))
                    // A stopped session is still worth listing and still starts
                    // on Enter, so it stays in place rather than being hidden or
                    // sorted away; dimming is what separates it from the hosts
                    // that are actually up.
                    .dimmed(!row.running)
            })
            .collect();
        let mut picker = ListPicker::new("Sessions · 1-9 attach · Tab actions", items)
            .with_column_header(
                format!("No. {:<name_width$}", "Name"),
                format!("{:<branch_width$}  {:<directory_width$}", "Branch", "Path"),
                "Last active",
            )
            .with_preview("Session");
        picker.primary_action = Some("attach".to_owned());
        picker.filter = filter;
        picker.selected = selected.min(self.workspace_rows.len().saturating_sub(1));
        self.list = Some(picker);
    }

    /// Advances visible elapsed values without rebuilding the manager on
    /// every host tick. Returns whether the snapshot changed.
    #[cfg(unix)]
    pub(crate) fn refresh_workspace_activity(&mut self) -> bool {
        self.refresh_workspace_activity_at(session_activity_now())
    }

    #[cfg(unix)]
    pub(super) fn refresh_workspace_activity_at(&mut self, now: u64) -> bool {
        let Some(picker) = self
            .list
            .as_ref()
            .filter(|picker| picker.title.starts_with("Sessions"))
        else {
            return false;
        };
        let changed = picker.items.len() != self.workspace_rows.len()
            || picker
                .items
                .iter()
                .zip(&self.workspace_rows)
                .any(|(item, row)| {
                    let last_active = if row.project_root == self.project_root {
                        Some(now)
                    } else {
                        row.last_active_unix_seconds
                    };
                    item.trailing_detail != compact_session_elapsed(last_active, now)
                });
        if changed {
            self.rebuild_workspace_picker_at(now);
        }
        changed
    }

    /// The branch column: the checked-out branch, or `-` for a detached
    /// checkout and for a workspace that is not a Git working tree at all.
    #[cfg(unix)]
    fn session_branch_cell(row: &crate::workspace::WorkspaceRow) -> String {
        row.git
            .as_ref()
            .and_then(|facts| facts.branch.clone())
            .unwrap_or_else(|| "-".to_owned())
    }

    /// The directory column: this workspace's own path, whether it is a linked
    /// Git worktree, a repository's main checkout, or not a repository at all.
    ///
    /// It remains the widest identity column even though the short activity
    /// column now follows it, so a path under the home directory is written
    /// with `~`. The preview keeps the full path either way.
    #[cfg(unix)]
    fn session_directory_cell(&self, row: &crate::workspace::WorkspaceRow) -> String {
        let path = &row.project_root;
        let relative = self
            .home_directory
            .as_deref()
            .and_then(|home| path.strip_prefix(home).ok());
        match relative {
            Some(relative) if relative.as_os_str().is_empty() => "~".to_owned(),
            Some(relative) => format!("~/{}", crate::git::display_path(relative)),
            None => crate::git::display_path(path),
        }
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

    /// Moves a confirmed teardown on, so the events that follow are read
    /// against the stage they belong to.
    #[cfg(unix)]
    pub(super) fn advance_worktree_teardown(&mut self, stage: super::WorktreeTeardownStage) {
        if let Some(teardown) = self.worktree_teardown.as_mut() {
            teardown.stage = stage;
        }
    }

    #[cfg(unix)]
    pub(super) fn start_session(&mut self, workspace: PathBuf) {
        if !self.persistent_session {
            self.action_failed("starting sessions needs workspace.mode: persistent");
            return;
        }
        self.workspace_generation = self.workspace_generation.wrapping_add(1).max(1);
        let generation = self.workspace_generation;
        let Some(service) = self.ports.workspace_service.as_ref() else {
            self.action_failed("session service is unavailable");
            return;
        };
        match service.try_start(generation, workspace, self.working_directory.clone()) {
            Ok(()) => self.status("starting session…"),
            Err(error) => self.error_from("Host", "Host operation failed", error),
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
            self.action_failed("stopping sessions needs workspace.mode: persistent");
            return;
        }
        let _ = self.request_session_stop(selector, force);
    }

    /// Stops a session without the `session` namespace's mode gate.
    ///
    /// That gate belongs to the commands somebody types, not to the host. A
    /// standalone editor still finds a persistent session running on a worktree
    /// it is removing — the session service is attached in either mode — and
    /// refusing to stop it there would abandon a removal already confirmed,
    /// halfway, for a reason that has nothing to do with the action.
    #[cfg(unix)]
    pub(super) fn request_session_stop(&mut self, selector: PathBuf, force: bool) -> Option<u64> {
        self.workspace_generation = self.workspace_generation.wrapping_add(1).max(1);
        let generation = self.workspace_generation;
        let Some(service) = self.ports.workspace_service.as_ref() else {
            self.action_failed("session service is unavailable");
            return None;
        };
        match service.try_stop(generation, selector, self.working_directory.clone(), force) {
            Ok(()) => {
                self.status(if force {
                    "force-stopping session and its protected live state…"
                } else {
                    "stopping session…"
                });
                Some(generation)
            }
            Err(error) => {
                self.error_from("Host", "Host operation failed", error);
                None
            }
        }
    }

    /// Drops a stopped session from the visited history behind the picker.
    #[cfg(unix)]
    pub(super) fn forget_workspace(&mut self, path: PathBuf) -> Option<u64> {
        self.workspace_generation = self.workspace_generation.wrapping_add(1).max(1);
        let generation = self.workspace_generation;
        let Some(service) = self.ports.workspace_service.as_ref() else {
            self.action_failed("session service is unavailable");
            return None;
        };
        match service.try_forget(generation, path) {
            Ok(()) => {
                self.status("forgetting session record…");
                Some(generation)
            }
            Err(error) => {
                self.error_from("Host", "Host operation failed", error);
                None
            }
        }
    }

    #[cfg(unix)]
    pub(super) fn rename_session(&mut self, path: PathBuf, name: String) {
        self.workspace_generation = self.workspace_generation.wrapping_add(1).max(1);
        let generation = self.workspace_generation;
        let Some(service) = self.ports.workspace_service.as_ref() else {
            self.action_failed("session service is unavailable");
            return;
        };
        match service.try_rename(generation, path, self.working_directory.clone(), name) {
            Ok(()) => self.status("renaming session…"),
            Err(error) => self.error_from("Host", "Host operation failed", error),
        }
    }

    /// Asks the catalog to give this workspace a number, or to take its away.
    #[cfg(unix)]
    pub(super) fn number_session(&mut self, path: PathBuf, number: Option<u8>) {
        self.workspace_generation = self.workspace_generation.wrapping_add(1).max(1);
        let generation = self.workspace_generation;
        let Some(service) = self.ports.workspace_service.as_ref() else {
            self.action_failed("session service is unavailable");
            return;
        };
        match service.try_number(generation, path, self.working_directory.clone(), number) {
            Ok(()) => self.status("numbering session…"),
            Err(error) => self.error_from("Host", "Host operation failed", error),
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

    /// Enables persistent-session lifecycle and workspace switching.
    pub fn enable_persistent_session(&mut self) {
        self.persistent_session = true;
    }
}

#[cfg(unix)]
fn session_activity_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
