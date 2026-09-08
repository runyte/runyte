// SPDX-License-Identifier: MPL-2.0

//! Persistent-session discovery, selection, preview, and lifecycle requests.

// Application-module dependencies:
use super::{App, InputGrammar, PathBuf, WorkspaceSwitchRequest};
#[cfg(unix)]
use super::{
    ListAction, ListPicker, PickerItem, WorkspaceEvent, WorkspaceServiceHandle,
    compact_session_elapsed, session_picker_preview, terminal_output_status,
};

#[cfg(unix)]
const SESSION_STATUS_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

impl App {
    #[cfg(unix)]
    pub fn attach_workspace_service(&mut self, service: WorkspaceServiceHandle) {
        self.ports.workspace_service = Some(service);
    }

    #[cfg(unix)]
    pub fn apply_workspace_event(&mut self, event: WorkspaceEvent) {
        match event {
            WorkspaceEvent::DirectoryWorktrees { generation, result } => {
                if generation == self.session_navigation.generation
                    && self.session_directory_chooser_open()
                    && let Ok(roots) = result
                {
                    self.session_navigation.worktrees = roots;
                    self.refresh_session_directory_chooser();
                }
            }
            WorkspaceEvent::Inventory {
                generation,
                path,
                result,
            } => {
                if !self
                    .session_navigation
                    .inventory
                    .as_ref()
                    .is_some_and(|inventory| {
                        inventory.generation == generation && inventory.path == path
                    })
                {
                    return;
                }
                match result {
                    Ok(inventory) => {
                        let items = inventory
                            .entries
                            .iter()
                            .enumerate()
                            .map(|(index, entry)| {
                                super::PickerItem::new(
                                    entry.label.clone(),
                                    entry.detail.clone(),
                                    index,
                                )
                            })
                            .collect();
                        self.list = Some(
                            super::ListPicker::fuzzy(
                                if inventory.truncated {
                                    "Session destinations · first 1024"
                                } else {
                                    "Session destinations"
                                },
                                items,
                            )
                            .as_manager(
                                "attach and visit",
                                "Escape",
                                "back",
                            ),
                        );
                        let state = self.session_navigation.inventory.as_mut().unwrap();
                        state.incarnation = Some(inventory.incarnation);
                        state.entries = inventory.entries;
                        if state.entries.is_empty() {
                            self.list = Some(
                                super::ListPicker::new(
                                    "Session destinations",
                                    vec![super::PickerItem::new(
                                        "No open destinations",
                                        "Escape returns to sessions",
                                        0,
                                    )],
                                )
                                .as_report(),
                            );
                        }
                    }
                    Err(error) => {
                        self.list = Some(
                            super::ListPicker::new(
                                "Session destinations · unavailable",
                                vec![super::PickerItem::new(
                                    error,
                                    "Escape returns to sessions",
                                    0,
                                )],
                            )
                            .as_report(),
                        )
                    }
                }
            }
            WorkspaceEvent::Observed { result } => {
                self.session_navigation.observation_pending = false;
                if std::mem::take(&mut self.session_navigation.observation_invalidated) {
                    // This scan began before the current attachment. Keep the
                    // retained rows until a scan of the new context finishes.
                    self.observe_session_strip();
                    return;
                }
                match result {
                    Ok(mut rows) => {
                        self.session_navigation.health_unknown = false;
                        for row in &mut rows {
                            row.git = self
                                .workspace_rows
                                .iter()
                                .find(|old| old.project_root == row.project_root)
                                .and_then(|old| old.git.clone());
                        }
                        let manager_open = self
                            .list
                            .as_ref()
                            .is_some_and(|picker| picker.title.starts_with("Sessions"));
                        if !manager_open && self.session_navigation.inventory.is_none() {
                            self.workspace_rows = rows;
                            if let Some(row) = self
                                .workspace_rows
                                .iter()
                                .find(|row| row.project_root == self.project_root)
                            {
                                self.note_workspace_number(row.number);
                            }
                        }
                        if self.session_directory_chooser_open() {
                            self.refresh_session_directory_chooser();
                        }
                        if let Some(next) = self.session_navigation.pending_cycle.take()
                            && self.workspace_rows.iter().any(|row| row.running)
                        {
                            self.cycle_persistent_session(next);
                        }
                    }
                    Err(error) => {
                        self.session_navigation.health_unknown = true;
                        for row in &mut self.workspace_rows {
                            if row.running {
                                row.interactive_attached = None;
                            }
                        }
                        if self.session_navigation.pending_cycle.take().is_some() {
                            self.action_failed(format!(
                                "cannot discover running sessions: {error}"
                            ));
                        }
                    }
                }
            }
            WorkspaceEvent::Inspected {
                generation,
                path,
                result,
            } => self.finish_worktree_session_check(generation, path, *result),
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
            WorkspaceEvent::Polled { result } => {
                let manager_open = self
                    .list
                    .as_ref()
                    .is_some_and(|picker| picker.title.starts_with("Sessions"));
                if !manager_open {
                    return;
                }
                // Routine observation stays silent on failure. The last
                // complete rows remain visible and the next bounded poll may
                // recover without turning a transient host into an error.
                if let Ok(rows) = result {
                    let selected_path = match self.selected_list_action() {
                        Some(ListAction::Workspace(index)) => self
                            .workspace_rows
                            .get(index)
                            .map(|row| row.project_root.clone()),
                        _ => None,
                    };
                    self.workspace_rows = rows;
                    if let Some(row) = self
                        .workspace_rows
                        .iter()
                        .find(|row| row.project_root == self.project_root)
                    {
                        self.note_workspace_number(row.number);
                    }
                    self.rebuild_workspace_picker();
                    if let Some(path) = selected_path
                        && let Some(row_index) = self
                            .workspace_rows
                            .iter()
                            .position(|row| row.project_root == path)
                        && let Some(picker) = self.list.as_mut()
                        && let Some(visible_index) = picker
                            .visible_indices()
                            .iter()
                            .position(|index| *index == row_index)
                    {
                        picker.selected = visible_index;
                    }
                    self.request_selected_workspace_preview();
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
            running_only: false,
            previous_session: false,
            visit: None,
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
            Ok(()) => {
                self.next_workspace_status_poll =
                    std::time::Instant::now() + SESSION_STATUS_POLL_INTERVAL;
                self.status("refreshing sessions…");
            }
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

        let (filter, selected, show_preview) = self
            .list
            .as_ref()
            .filter(|picker| picker.title.starts_with("Sessions"))
            .map_or_else(
                || (String::new(), 0, true),
                |picker| {
                    let selected = if picker.title == "Sessions · loading…" {
                        self.workspace_rows
                            .iter()
                            .position(|row| row.project_root == self.project_root)
                            .unwrap_or(0)
                    } else {
                        picker.selected
                    };
                    (picker.filter.clone(), selected, picker.show_preview)
                },
            );
        self.list_actions = (0..self.workspace_rows.len())
            .map(ListAction::Workspace)
            .collect();
        // The manager reads as six columns — number, name, branch, directory,
        // last activity, and protected-work/terminal-output status —
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
                    terminal_output_status(row, now),
                )
            })
            .collect::<Vec<_>>();
        let name_width = columns
            .iter()
            .map(|(name, _, _, _, _)| name.width())
            .max()
            .unwrap_or(0)
            .max("Name".width());
        let branch_width = columns
            .iter()
            .map(|(_, branch, _, _, _)| branch.width())
            .max()
            .unwrap_or(0)
            .max("Branch".width());
        let directory_width = columns
            .iter()
            .map(|(_, _, directory, _, _)| directory.width())
            .max()
            .unwrap_or(0)
            .max("Path".width());
        let activity_width = columns
            .iter()
            .map(|(_, _, _, active, _)| active.width())
            .max()
            .unwrap_or(0)
            .max("Last active".width());
        let status_width = columns
            .iter()
            .map(|(_, _, _, _, status)| status.width())
            .max()
            .unwrap_or(0)
            .max(6);
        let items = self
            .workspace_rows
            .iter()
            .zip(columns.iter())
            .enumerate()
            .map(
                |(index, (row, (name, branch, directory, active, status)))| {
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
                        .with_trailing_detail(format!(
                            "{active:<activity_width$}  {status:<status_width$}"
                        ))
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
                },
            )
            .collect();
        let mut picker = ListPicker::new("Sessions · 1-9 attach · Tab actions", items)
            .with_column_header(
                format!("No. {:<name_width$}", "Name"),
                format!("{:<branch_width$}  {:<directory_width$}", "Branch", "Path"),
                format!(
                    "{:<activity_width$}  {:<status_width$}",
                    "Last active", "Status"
                ),
            )
            .with_preview("Session");
        picker.primary_action = Some("attach".to_owned());
        picker.filter = filter;
        picker.selected = selected.min(self.workspace_rows.len().saturating_sub(1));
        picker.show_preview = show_preview;
        self.list = Some(picker);
    }

    /// Advances visible elapsed values without rebuilding the manager on
    /// every host tick. Returns whether the snapshot changed.
    #[cfg(unix)]
    pub(crate) fn refresh_workspace_activity(&mut self) -> bool {
        let changed = self.refresh_workspace_activity_at(session_activity_now());
        self.poll_workspace_statuses_at(std::time::Instant::now());
        self.observe_session_strip();
        changed
    }

    #[cfg(unix)]
    fn poll_workspace_statuses_at(&mut self, now: std::time::Instant) {
        if now < self.next_workspace_status_poll
            || !self
                .list
                .as_ref()
                .is_some_and(|picker| picker.title.starts_with("Sessions"))
        {
            return;
        }
        self.next_workspace_status_poll = now + SESSION_STATUS_POLL_INTERVAL;
        if let Some(service) = self.ports.workspace_service.as_ref() {
            let _ = service.try_poll();
        }
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
        let activity_width = self
            .workspace_rows
            .iter()
            .map(|row| {
                let last_active = if row.project_root == self.project_root {
                    Some(now)
                } else {
                    row.last_active_unix_seconds
                };
                unicode_width::UnicodeWidthStr::width(
                    compact_session_elapsed(last_active, now).as_str(),
                )
            })
            .max()
            .unwrap_or(0)
            .max(unicode_width::UnicodeWidthStr::width("Last active"));
        let status_width = self
            .workspace_rows
            .iter()
            .map(|row| unicode_width::UnicodeWidthStr::width(terminal_output_status(row, now)))
            .max()
            .unwrap_or(0)
            .max(6);
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
                    item.trailing_detail
                        != format!(
                            "{:<activity_width$}  {:<status_width$}",
                            compact_session_elapsed(last_active, now),
                            terminal_output_status(row, now),
                            activity_width = activity_width,
                        )
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

/// UI observation is independent from manager request generations and does not
/// retain documents, terminals, or remote contents.
#[derive(Default)]
pub(super) struct SessionNavigationState {
    observation_pending: bool,
    observation_invalidated: bool,
    next_observation: Option<std::time::Instant>,
    health_unknown: bool,
    directory: Option<SessionDirectoryChooser>,
    pending_cycle: Option<bool>,
    generation: u64,
    worktrees: Vec<PathBuf>,
    inventory: Option<SessionInventory>,
}

struct SessionInventory {
    path: PathBuf,
    generation: u64,
    saved_picker: super::ListPicker,
    incarnation: Option<String>,
    entries: Vec<crate::protocol::OpenDestinationEntry>,
}

struct SessionDirectoryChooser {
    root: PathBuf,
    query: String,
    paths: Vec<PathBuf>,
    previous_mode: super::Mode,
}

impl App {
    pub(super) fn session_directory_chooser_open(&self) -> bool {
        self.session_navigation.directory.is_some()
    }

    pub(super) fn open_session_directory_chooser(&mut self) {
        if self.reject_unavailable_persistent_session(cfg!(unix), true) {
            return;
        }
        if !self.persistent_session {
            self.action_failed("opening a persistent session needs workspace.mode: persistent");
            return;
        }
        self.session_navigation.directory = Some(SessionDirectoryChooser {
            root: self.working_directory.clone(),
            query: String::new(),
            paths: Vec::new(),
            previous_mode: self.mode,
        });
        self.session_action_menu = None;
        self.grammar.reset();
        self.session_navigation.generation = self.session_navigation.generation.wrapping_add(1);
        self.session_navigation.worktrees.clear();
        #[cfg(unix)]
        if let Some(service) = self.ports.workspace_service.as_ref() {
            let _ = service.try_directory_worktrees(
                self.session_navigation.generation,
                self.project_root.clone(),
            );
        }
        self.refresh_session_directory_chooser();
    }

    pub(super) fn insert_session_directory_text(&mut self, text: &str) {
        if let Some(chooser) = self.session_navigation.directory.as_mut() {
            chooser.query.push_str(text);
            self.rebuild_session_directory_chooser();
        }
    }

    fn refresh_session_directory_chooser(&mut self) {
        let selected = self
            .list
            .as_ref()
            .and_then(|picker| picker.selected_item())
            .and_then(|item| {
                self.session_navigation
                    .directory
                    .as_ref()?
                    .paths
                    .get(item.index)
            })
            .cloned();
        self.rebuild_session_directory_chooser();
        if let Some(path) = selected {
            let index = self
                .session_navigation
                .directory
                .as_ref()
                .and_then(|chooser| {
                    chooser
                        .paths
                        .iter()
                        .position(|candidate| candidate == &path)
                });
            if let (Some(index), Some(picker)) = (index, self.list.as_mut())
                && let Some(selected) = picker
                    .visible_indices()
                    .iter()
                    .position(|visible| picker.items[*visible].index == index)
            {
                picker.selected = selected;
            }
        }
    }

    fn rebuild_session_directory_chooser(&mut self) {
        use super::{ListPicker, PickerItem};
        let Some(chooser) = self.session_navigation.directory.as_ref() else {
            return;
        };
        let mut root = chooser.root.clone();
        let query = chooser.query.clone();
        let mut filter = query.clone();
        if query.contains('/') || query.starts_with('~') {
            let expanded = if query == "~" || query.starts_with("~/") {
                self.home_directory
                    .as_ref()
                    .map(|home| home.join(query.trim_start_matches('~').trim_start_matches('/')))
                    .unwrap_or_else(|| PathBuf::from(&query))
            } else {
                PathBuf::from(&query)
            };
            let absolute = if expanded.is_absolute() {
                expanded
            } else {
                root.join(expanded)
            };
            if query.ends_with('/') || query == "~" {
                root = absolute;
                filter.clear();
            } else if let Some(parent) = absolute.parent() {
                root = parent.to_path_buf();
                filter = absolute
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
            }
        }
        let mut paths = vec![root.clone()];
        let mut labels = vec!["Open this directory".to_owned()];
        if let Some(parent) = root.parent() {
            paths.push(parent.to_path_buf());
            labels.push(".. · parent directory".to_owned());
        }
        let children_start = paths.len();
        if let Some(entries) = self.path_listings.borrow_mut().read(&root) {
            for entry in entries.iter().filter(|entry| entry.is_directory) {
                paths.push(root.join(&entry.name));
                labels.push(format!("{}/", entry.name));
            }
        }
        let children = children_start..paths.len();
        #[cfg(unix)]
        if query.is_empty() {
            for row in &self.workspace_rows {
                if !paths.contains(&row.project_root) {
                    paths.push(row.project_root.clone());
                    labels.push(format!("{} · recent root", row.display_name()));
                }
            }
            for path in &self.session_navigation.worktrees {
                if !paths.contains(path) {
                    paths.push(path.clone());
                    labels.push("Git worktree".to_owned());
                }
            }
        }
        let items = paths
            .iter()
            .zip(labels)
            .enumerate()
            .map(|(index, (path, label))| PickerItem::new(label, path.display().to_string(), index))
            .collect();
        let mut picker = ListPicker::fuzzy(format!("Open directory · {}", root.display()), items)
            .as_manager("open persistent session", "Tab", "browse directory");
        picker.filter = filter;
        // Rank the whole directory before limiting child results. Root, parent,
        // recent-root and worktree shortcuts keep their own places.
        let visible = picker
            .visible_indices()
            .into_iter()
            .filter(|index| children.contains(index))
            .take(256)
            .collect::<std::collections::HashSet<_>>();
        picker
            .items
            .retain(|item| !children.contains(&item.index) || visible.contains(&item.index));
        self.list = Some(picker);
        self.list_actions.clear();
        self.session_navigation.directory.as_mut().unwrap().paths = paths;
    }

    pub(super) fn handle_session_directory_key(
        &mut self,
        key: super::KeyStroke,
    ) -> super::Result<()> {
        use super::{KeyCode, Modifiers};
        let control = key.modifiers.contains(Modifiers::CONTROL);
        match (key.code, control) {
            (KeyCode::Escape, _) | (KeyCode::Char('c'), true) => {
                let chooser = self.session_navigation.directory.take().unwrap();
                self.mode = chooser.previous_mode;
                self.list = None;
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) | (KeyCode::BackTab, _) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.up();
                }
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.down();
                }
            }
            (KeyCode::PageUp, _) | (KeyCode::Char('u'), true) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.page_up(10);
                }
            }
            (KeyCode::PageDown, _) | (KeyCode::Char('d'), true) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.page_down(10);
                }
            }
            (KeyCode::Home, _) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.first();
                }
            }
            (KeyCode::End, _) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.last();
                }
            }
            (KeyCode::Tab, _) | (KeyCode::Enter, _) => {
                let path = self
                    .list
                    .as_ref()
                    .and_then(|picker| picker.selected_item())
                    .and_then(|item| {
                        self.session_navigation
                            .directory
                            .as_ref()?
                            .paths
                            .get(item.index)
                    })
                    .cloned();
                if let Some(path) = path {
                    if !path.is_dir() {
                        self.action_failed("that directory is no longer available");
                        return Ok(());
                    }
                    if key.code == KeyCode::Tab {
                        let chooser = self.session_navigation.directory.as_mut().unwrap();
                        chooser.root = path;
                        chooser.query.clear();
                        self.rebuild_session_directory_chooser();
                    } else if self.request_workspace_switch(path) {
                        self.session_navigation.directory = None;
                        self.list = None;
                    }
                }
            }
            (KeyCode::Backspace, _) => {
                self.session_navigation
                    .directory
                    .as_mut()
                    .unwrap()
                    .query
                    .pop();
                self.rebuild_session_directory_chooser();
            }
            (KeyCode::Char(character), false) if !key.modifiers.contains(Modifiers::ALT) => {
                self.session_navigation
                    .directory
                    .as_mut()
                    .unwrap()
                    .query
                    .push(character);
                self.rebuild_session_directory_chooser();
            }
            _ => {}
        }
        Ok(())
    }

    /// Shared numbered-session action for the leader and the session manager.
    pub(super) fn attach_numbered_session(&mut self, digit: char) {
        if !self.persistent_session {
            self.action_failed("attaching sessions needs workspace.mode: persistent");
            return;
        }
        #[cfg(unix)]
        {
            let Some(number) = digit.to_digit(10).map(|number| number as u8) else {
                return;
            };
            let Some(path) = self
                .workspace_rows
                .iter()
                .find(|row| row.running && row.number == Some(number))
                .map(|row| row.project_root.clone())
            else {
                self.action_failed(format!("no session is numbered {number}"));
                return;
            };
            self.list = None;
            self.session_action_menu = None;
            if self.request_workspace_switch(path) {
                self.workspace_switch.as_mut().unwrap().running_only = true;
                self.should_quit = true;
            }
        }
        #[cfg(not(unix))]
        let _ = digit;
    }

    pub(super) fn previous_persistent_session(&mut self) {
        if self.request_workspace_switch(self.project_root.clone()) {
            let request = self.workspace_switch.as_mut().unwrap();
            request.previous_session = true;
            request.running_only = true;
        }
    }

    pub(super) fn cycle_persistent_session(&mut self, next: bool) {
        #[cfg(unix)]
        {
            if !self.persistent_session {
                self.action_failed("session navigation needs workspace.mode: persistent");
                return;
            }
            let roots = self
                .workspace_rows
                .iter()
                .filter(|row| row.running)
                .map(|row| row.project_root.clone())
                .collect::<Vec<_>>();
            if roots.is_empty() {
                if self.ports.workspace_service.is_none() {
                    self.action_failed("session service is unavailable");
                    return;
                }
                self.session_navigation.pending_cycle = Some(next);
                self.session_navigation.next_observation = None;
                self.observe_session_strip();
                return;
            }
            if roots.len() == 1 && roots[0] == self.project_root {
                return;
            }
            let current = roots.iter().position(|root| root == &self.project_root);
            let index = match (current, next) {
                (Some(index), true) => (index + 1) % roots.len(),
                (Some(index), false) => (index + roots.len() - 1) % roots.len(),
                (None, true) => 0,
                (None, false) => roots.len() - 1,
            };
            if self.request_workspace_switch(roots[index].clone()) {
                self.workspace_switch.as_mut().unwrap().running_only = true;
            }
        }
        #[cfg(not(unix))]
        {
            let _ = next;
            self.action_failed("persistent sessions are unavailable on this platform");
        }
    }

    #[cfg(unix)]
    pub(super) fn refresh_sessions_on_attachment(&mut self) {
        self.session_navigation.next_observation = None;
        self.session_navigation.observation_invalidated =
            self.session_navigation.observation_pending;
        self.next_workspace_status_poll = std::time::Instant::now();
        self.refresh_workspace_activity();
    }

    #[cfg(unix)]
    fn observe_session_strip(&mut self) {
        let now = std::time::Instant::now();
        if !self.persistent_session
            || self
                .list
                .as_ref()
                .is_some_and(|picker| picker.title.starts_with("Sessions"))
            || self.session_navigation.observation_pending
            || self
                .session_navigation
                .next_observation
                .is_some_and(|next| now < next)
        {
            return;
        }
        self.session_navigation.next_observation = Some(now + std::time::Duration::from_secs(15));
        if let Some(service) = self.ports.workspace_service.as_ref() {
            self.session_navigation.observation_pending = service
                .try_observe(self.session_strip_snapshot().is_some())
                .is_ok();
        }
    }

    pub(super) fn prepare_session_strip(
        &self,
        mut geometry: super::FrameGeometry,
    ) -> (
        super::FrameGeometry,
        Option<crate::session_strip::PreparedSessionStrip>,
    ) {
        let strip = self
            .session_strip_snapshot()
            .filter(|_| geometry.editor.height > 0)
            .map(|snapshot| {
                #[cfg(unix)]
                let targets = {
                    let mut targets = self
                        .workspace_rows
                        .iter()
                        .filter(|row| row.running)
                        .map(|row| row.project_root.clone())
                        .collect::<Vec<_>>();
                    if !targets.contains(&self.project_root) {
                        targets.push(self.project_root.clone());
                    }
                    targets
                };
                #[cfg(not(unix))]
                let targets = Vec::new();
                crate::session_strip::PreparedSessionStrip { snapshot, targets }
            });
        if strip.is_some() {
            geometry.editor.y = geometry.editor.y.saturating_add(1);
            geometry.editor.height -= 1;
        }
        (geometry, strip)
    }

    pub fn session_strip_snapshot(&self) -> Option<crate::snapshot::SessionStripSnapshot> {
        #[cfg(unix)]
        {
            use crate::{
                config::SessionStripVisibility,
                snapshot::{SessionStripEntry, SessionStripSnapshot},
            };
            if !self.persistent_session
                || self.config.workspace.session_strip == SessionStripVisibility::Hidden
                || self
                    .maximized
                    .is_some_and(|view| view.view == super::MaximizedView::Zen)
            {
                return None;
            }
            let mut entries = self
                .workspace_rows
                .iter()
                .filter(|row| row.running)
                .map(|row| SessionStripEntry {
                    name: row.display_name(),
                    number: row.number,
                    current: row.project_root == self.project_root,
                    unread: row.unread_terminals.is_some_and(|count| count > 0),
                    bell: row.terminal_bell.unwrap_or(false),
                    health_unknown: self.session_navigation.health_unknown
                        || row.incompatible_protocol.is_some()
                        || row.interactive_attached.is_none(),
                })
                .collect::<Vec<_>>();
            if !entries.iter().any(|entry| entry.current) {
                entries.push(SessionStripEntry {
                    name: self.project_root.file_name().map_or_else(
                        || self.project_root.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    ),
                    number: self.workspace_number,
                    current: true,
                    unread: false,
                    bell: false,
                    health_unknown: false,
                });
            }
            if self.config.workspace.session_strip == SessionStripVisibility::Auto
                && entries.len() < 2
            {
                return None;
            }
            Some(SessionStripSnapshot { entries })
        }
        #[cfg(not(unix))]
        {
            None
        }
    }
}

impl App {
    pub(super) fn session_inventory_open(&self) -> bool {
        self.session_navigation.inventory.is_some()
    }

    pub(super) fn open_session_inventory(&mut self) {
        #[cfg(unix)]
        {
            let Some(ListAction::Workspace(index)) = self.selected_list_action() else {
                return;
            };
            let Some(row) = self.workspace_rows.get(index).cloned() else {
                return;
            };
            let Some(saved_picker) = self.list.take() else {
                return;
            };
            self.session_navigation.generation = self.session_navigation.generation.wrapping_add(1);
            let generation = self.session_navigation.generation;
            self.session_navigation.inventory = Some(SessionInventory {
                path: row.project_root.clone(),
                generation,
                saved_picker,
                incarnation: None,
                entries: Vec::new(),
            });
            self.session_action_menu = None;
            let message = if !row.running {
                "This session is stopped"
            } else if row.incompatible_protocol.is_some() {
                "This host uses an unsupported protocol"
            } else {
                "Loading open destinations…"
            };
            self.list = Some(
                ListPicker::new(
                    "Session destinations",
                    vec![PickerItem::new(message, "Escape returns to sessions", 0)],
                )
                .as_report(),
            );
            if row.running && row.incompatible_protocol.is_none() {
                let result = self
                    .ports
                    .workspace_service
                    .as_ref()
                    .ok_or("session service is unavailable")
                    .and_then(|service| service.try_inventory(generation, row.project_root));
                if let Err(error) = result {
                    self.list = Some(
                        ListPicker::new(
                            "Session destinations · unavailable",
                            vec![PickerItem::new(error, "Escape returns to sessions", 0)],
                        )
                        .as_report(),
                    );
                }
            }
        }
        #[cfg(not(unix))]
        self.action_failed("persistent sessions are unavailable on this platform");
    }

    pub(super) fn handle_session_inventory_key(
        &mut self,
        key: super::KeyStroke,
    ) -> super::Result<()> {
        use super::{KeyCode, Modifiers};
        let control = key.modifiers.contains(Modifiers::CONTROL);
        match (key.code, control) {
            (KeyCode::Escape, _) | (KeyCode::Char('c'), true) => {
                let inventory = self.session_navigation.inventory.take().unwrap();
                self.list = Some(inventory.saved_picker);
                #[cfg(unix)]
                {
                    self.rebuild_workspace_picker();
                    if let Some(index) = self
                        .workspace_rows
                        .iter()
                        .position(|row| row.project_root == inventory.path)
                        && let Some(picker) = self.list.as_mut()
                        && let Some(selected) = picker
                            .visible_indices()
                            .iter()
                            .position(|visible| picker.items[*visible].index == index)
                    {
                        picker.selected = selected;
                    }
                    self.request_selected_workspace_preview();
                }
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) | (KeyCode::BackTab, _) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.up();
                }
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.down();
                }
            }
            (KeyCode::PageUp, _) | (KeyCode::Char('u'), true) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.page_up(10);
                }
            }
            (KeyCode::PageDown, _) | (KeyCode::Char('d'), true) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.page_down(10);
                }
            }
            (KeyCode::Home, _) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.first();
                }
            }
            (KeyCode::End, _) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.last();
                }
            }
            (KeyCode::Enter, _) => {
                let inventory = self.session_navigation.inventory.as_ref().unwrap();
                let selected = self
                    .list
                    .as_ref()
                    .and_then(|picker| picker.selected_item())
                    .and_then(|item| inventory.entries.get(item.index));
                if let (Some(entry), Some(incarnation)) = (selected, inventory.incarnation.clone())
                {
                    let destination = match entry.destination {
                        crate::protocol::OpenDestination::Buffer(id) => {
                            let Some(index) = id
                                .checked_sub(1)
                                .and_then(|index| usize::try_from(index).ok())
                            else {
                                self.action_failed("invalid destination identity");
                                return Ok(());
                            };
                            super::OpenDestination::Buffer(index)
                        }
                        crate::protocol::OpenDestination::Terminal(id) => {
                            super::OpenDestination::Terminal(super::TerminalId::from_raw(id))
                        }
                    };
                    let path = inventory.path.clone();
                    if self.request_workspace_switch(path) {
                        let request = self.workspace_switch.as_mut().unwrap();
                        request.running_only = true;
                        request.visit = Some(super::DestinationVisit {
                            incarnation,
                            destination,
                        });
                        self.session_navigation.inventory = None;
                        self.list = None;
                    }
                }
            }
            (KeyCode::Backspace, _) => {
                if let Some(picker) = self.list.as_mut() {
                    picker.pop_filter();
                }
            }
            (KeyCode::Char(character), false) if !key.modifiers.contains(Modifiers::ALT) => {
                if let Some(picker) = self.list.as_mut()
                    && picker.purpose != super::ListPurpose::Report
                {
                    picker.push_filter(character);
                }
            }
            _ => {}
        }
        Ok(())
    }
}
