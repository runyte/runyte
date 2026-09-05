// SPDX-License-Identifier: MPL-2.0

//! Settings, themes, notifications, service health, and lifecycle feedback.

// Application-module dependencies:
use super::{
    ActionFeedback, ActiveGrammar, App, Buffer, CommandOutcome, DefaultColors, FailureClass,
    InputGrammar, ListAction, ListPicker, Mode, NotificationCenter, NotificationCounts,
    NotificationDraft, NotificationSeverity, Path, PickerItem, PreviewPolicy, PromptKind, Result,
    Selection, ServiceHealthEntry, ServiceHealthSnapshot, ServiceState, SettingId, SettingPreview,
    SettingType, SettingValue, SettingsView, Theme, ThemeAppearance, WorkspaceMode, fs,
    outcome_clause, persist_setting, registry_failure_summary, render_settings_page,
    startup_status,
};
use crate::{
    buffer::GeneratedViewIdentity, config::ExplorerSort, content_alignment::ContentAlignment,
};

impl App {
    /// Returns a complete optional-service report without starting a provider
    /// or requiring any of the reported services to be present.
    pub fn service_health_snapshot(&self) -> ServiceHealthSnapshot {
        self.service_health_with_environment()
    }

    fn service_health_with_environment(&self) -> ServiceHealthSnapshot {
        let mut entries = Vec::new();
        let syntax_errors = self.registry.errors();
        let active_syntax = self
            .syntax
            .get(self.active().buffer)
            .is_some_and(Option::is_some);
        if syntax_errors.is_empty() {
            entries.push(ServiceHealthEntry::new(
                "syntax",
                if active_syntax {
                    ServiceState::Ready
                } else {
                    ServiceState::Idle
                },
                if active_syntax {
                    "active buffer parsed successfully"
                } else {
                    "active buffer is using plain text"
                },
            ));
        } else {
            for error in syntax_errors {
                entries.push(ServiceHealthEntry::new(
                    "syntax",
                    ServiceState::Degraded,
                    error.to_string(),
                ));
            }
        }

        let buffer_id = self.active().buffer;
        let language = self.language_of(buffer_id);
        let (lsp_state, lsp_detail) = if !self.config.lsp.enable {
            (ServiceState::Disabled, "disabled in settings".to_owned())
        } else if !self.ports.has_lsp() {
            (
                ServiceState::Unavailable,
                "language-server manager is not attached".to_owned(),
            )
        } else if let Some(language) = language {
            if !self.config.lsp.servers.contains_key(&language) {
                (
                    ServiceState::Idle,
                    format!("no server configured for active {language} buffer"),
                )
            } else if self.lsp_servers.contains_key(&language)
                && self.lsp_documents.contains_key(&buffer_id)
            {
                (
                    ServiceState::Ready,
                    format!("{language} server and document are attached"),
                )
            } else {
                (
                    ServiceState::Idle,
                    format!("{language} server is configured and starting or stopped"),
                )
            }
        } else {
            (
                ServiceState::Idle,
                "active buffer has no recognized language".to_owned(),
            )
        };
        entries.push(ServiceHealthEntry::new("lsp", lsp_state, lsp_detail));

        entries.push(self.logging_health());
        ServiceHealthSnapshot { entries }
    }

    /// Describes the diagnostic log of the process that owns this `App`.
    ///
    /// In persistent mode that is the host, so a newly attached client sees
    /// how the process holding its workspace is actually logging rather than
    /// the flags its own launch happened to carry.
    fn logging_health(&self) -> ServiceHealthEntry {
        logging_health_entry(crate::log::status())
    }

    /// Opens the log this process owns as an ordinary read-only buffer.
    ///
    /// It reads whichever file the installed logger owns, so in persistent
    /// mode the host's `host.log` is what a client sees. No client-side trace
    /// is opened or aggregated.
    pub(super) fn open_log_buffer(&mut self) {
        let Some(status) = crate::log::status() else {
            self.action_failed("no diagnostic log is installed for this process");
            return;
        };
        let Some(path) = status.path else {
            self.action_failed("this process's diagnostic log has no file destination");
            return;
        };
        // The queue is drained before reading so the records that explain what
        // just happened are already on disk.
        crate::log::flush(crate::log::FLUSH_BUDGET);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                self.error_from(
                    "Runyte",
                    "Diagnostic log read failed",
                    format!("cannot read {}: {error}", path.display()),
                );
                return;
            }
        };
        let header = format!(
            "{} · {} owner · {}

",
            path.display(),
            status.role,
            status.level.map_or_else(
                || "not recording".to_owned(),
                |level| level.label().to_ascii_lowercase()
            )
        );
        self.open_virtual_page(
            GeneratedViewIdentity::Log,
            crate::buffer::LOG_NAME.to_owned(),
            &format!("{header}{text}"),
            ContentAlignment::default(),
        );
    }

    pub(super) fn open_service_health(&mut self) {
        let report = self.service_health_snapshot();
        let items = report
            .entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                PickerItem::new(
                    entry.service,
                    format!("{} · {}", entry.state.label(), entry.detail),
                    index,
                )
            })
            .collect();
        self.list_actions.clear();
        self.settings_view = None;
        self.list = Some(ListPicker::new("Service health", items).as_report());
    }

    pub(super) fn effective_setting_value(&self, setting: SettingId) -> SettingValue {
        match setting {
            SettingId::EditorGrammar => SettingValue::Grammar(self.grammar.kind()),
            SettingId::Theme => SettingValue::Text(self.theme_name.clone()),
            SettingId::LspEnable => SettingValue::Boolean(self.config.lsp.enable),
            SettingId::GitRefreshIntervalSeconds => {
                SettingValue::Integer(self.config.git.refresh_interval_seconds)
            }
            SettingId::WorkspaceMode => SettingValue::WorkspaceMode(self.config.workspace.mode),
            _ => setting.configured_value(&self.config),
        }
    }

    fn persisted_setting_label(&self, setting: SettingId) -> String {
        if setting == SettingId::Theme && self.persisted_config.theme.is_none() {
            format!(
                "default ({})",
                setting.configured_value(&self.persisted_config)
            )
        } else {
            setting.configured_value(&self.persisted_config).to_string()
        }
    }

    pub(super) fn settings_buffer(&self) -> Buffer {
        let values = SettingId::ALL
            .iter()
            .copied()
            .map(|setting| (setting, self.persisted_setting_label(setting)))
            .collect::<Vec<_>>();
        let page = render_settings_page(&values);
        Buffer::settings(&page.text, page.rows)
    }

    pub(super) fn open_settings_buffer(&mut self) {
        let rendered = self.settings_buffer();
        let buffer = match self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_settings()).then_some(index)
        }) {
            Some(existing) => {
                self.buffers[existing] = rendered;
                self.normalize_buffer(existing);
                existing
            }
            None => {
                self.buffers.push(rendered);
                self.syntax.push(None);
                self.buffers.len() - 1
            }
        };
        self.push_jump();
        let pane = self.active_mut();
        pane.retarget(buffer);
        pane.replace_selection(Selection::point(0));
        pane.scroll_row = 0;
        pane.scroll_wrap = 0;
        pane.scroll_col = 0;
        pane.preserve_scroll = false;
        self.mode = Mode::Normal;
        self.status("config · Enter changes the setting on this row");
    }

    pub(super) fn open_notifications_buffer(&mut self) {
        self.notifications.acknowledge();
        let rendered = Buffer::notifications(self.notifications.render());
        let buffer = match self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index) && buffer.is_notifications()).then_some(index)
        }) {
            Some(existing) => {
                self.buffers[existing] = rendered;
                self.normalize_buffer(existing);
                existing
            }
            None => {
                self.buffers.push(rendered);
                self.syntax.push(None);
                self.buffers.len() - 1
            }
        };
        self.push_jump();
        let pane = self.active_mut();
        pane.retarget(buffer);
        pane.replace_selection(Selection::point(0));
        pane.scroll_row = 0;
        pane.scroll_wrap = 0;
        pane.scroll_col = 0;
        pane.preserve_scroll = false;
        self.mode = Mode::Normal;
    }

    fn refresh_notification_buffers(&mut self) {
        let open = self
            .buffers
            .iter()
            .enumerate()
            .filter_map(|(index, buffer)| {
                (!self.closed_buffers.contains(&index) && buffer.is_notifications())
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        if open.is_empty() {
            return;
        }
        let rendered = Buffer::notifications(self.notifications.render());
        for index in open {
            self.buffers[index] = rendered.clone();
            self.normalize_buffer(index);
        }
    }

    pub fn unread_notification_counts(&self) -> NotificationCounts {
        self.notifications.unread_counts()
    }

    pub fn notifications(&self) -> &NotificationCenter {
        &self.notifications
    }

    pub fn push_notification(&mut self, notification: NotificationDraft) {
        self.notifications.push(notification);
        self.refresh_notification_buffers();
    }

    pub(super) fn refresh_settings_buffers(&mut self) {
        let rendered = self.settings_buffer();
        for index in 0..self.buffers.len() {
            if self.buffers[index].is_settings() {
                self.buffers[index] = rendered.clone();
                self.normalize_buffer(index);
            }
        }
    }

    pub(super) fn activate_selected_setting(&mut self) {
        let row = self.cursor_position().row;
        let Some(setting) = self.active_buffer().setting_at(row) else {
            self.status("no setting on this row");
            return;
        };
        self.open_setting_values(setting);
    }

    pub(super) fn setting_values(&self, setting: SettingId) -> Vec<SettingValue> {
        match setting.descriptor().value_type {
            SettingType::Grammar => crate::command::GrammarKind::ALL
                .iter()
                .copied()
                .map(SettingValue::Grammar)
                .collect(),
            SettingType::Boolean => vec![SettingValue::Boolean(true), SettingValue::Boolean(false)],
            SettingType::Theme => setting
                .allowed_values(&self.config)
                .into_iter()
                .map(SettingValue::Text)
                .collect(),
            SettingType::WorkspaceMode => WorkspaceMode::ALL
                .iter()
                .copied()
                .map(SettingValue::WorkspaceMode)
                .collect(),
            SettingType::ExplorerSort => ExplorerSort::ALL
                .iter()
                .copied()
                .map(SettingValue::ExplorerSort)
                .collect(),
            SettingType::Integer { minimum, maximum } => {
                (minimum..=maximum).map(SettingValue::Integer).collect()
            }
            SettingType::Text => Vec::new(),
        }
    }

    /// Which Tab-cycled group a choice belongs to, or `None` when the setting
    /// has no such axis. Only themes do: they divide into the dark and the
    /// light ones, and the list is long enough that reading it is easier one
    /// half at a time.
    fn setting_value_group(&self, setting: SettingId, value: &SettingValue) -> Option<String> {
        if setting.descriptor().value_type != SettingType::Theme {
            return None;
        }
        let SettingValue::Text(name) = value else {
            return None;
        };
        Some(
            self.config
                .resolve_theme(name)
                .ok()?
                .appearance()?
                .to_string(),
        )
    }

    pub(super) fn open_setting_values(&mut self, setting: SettingId) {
        if matches!(
            setting.descriptor().value_type,
            SettingType::Integer { .. } | SettingType::Text
        ) {
            self.list = None;
            self.list_actions.clear();
            self.settings_view = None;
            self.open_prompt(PromptKind::SettingValue(setting));
            self.command = self.effective_setting_value(setting).to_string();
            self.command_cursor = self.command.chars().count();
            return;
        }
        let values = self.setting_values(setting);
        if values.is_empty() {
            self.action_failed(format!(
                "{} has no valid choices in the loaded configuration",
                setting.descriptor().title
            ));
            return;
        }
        let effective = self.effective_setting_value(setting);
        let saved = self.persisted_setting_label(setting);
        let items = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let marker = if *value == effective {
                    "effective"
                } else if value.to_string() == saved {
                    "saved"
                } else {
                    "choice"
                };
                let item = PickerItem::new(value.to_string(), marker, index);
                match self.setting_value_group(setting, value) {
                    Some(group) => item.with_tag(group),
                    None => item,
                }
            })
            .collect();
        self.list_actions = values
            .into_iter()
            .map(|value| ListAction::SettingValue { setting, value })
            .collect();
        let mut picker = ListPicker::new(setting.descriptor().key, items).as_choice("to save");
        if setting.descriptor().value_type == SettingType::Theme {
            picker = picker.with_tags(vec![
                ThemeAppearance::Dark.to_string(),
                ThemeAppearance::Light.to_string(),
            ]);
        }
        picker.selected = self
            .list_actions
            .iter()
            .position(|action| {
                matches!(
                    action,
                    ListAction::SettingValue { value, .. } if *value == effective
                )
            })
            .unwrap_or(0);
        self.settings_view = Some(SettingsView::Values(Box::new(SettingPreview {
            setting,
            original_config: self.config.clone(),
            original_theme: self.theme.clone(),
            original_theme_name: self.theme_name.clone(),
            original_grammar: self.grammar.clone(),
            original_mode: self.mode,
        })));
        self.list = Some(picker);
        self.preview_selected_setting_value();
    }

    pub(super) fn preview_selected_setting_value(&mut self) {
        let Some(SettingsView::Values(preview)) = self.settings_view.as_ref() else {
            return;
        };
        let setting = preview.setting;
        let Some(ListAction::SettingValue { value, .. }) = self.selected_list_action() else {
            return;
        };
        if setting.descriptor().preview == PreviewPolicy::RestartRequired {
            self.status(format!(
                "{}: {value} · Enter saves · restart required to apply",
                setting.descriptor().title
            ));
            return;
        }
        if let Err(error) = setting.apply(&value, &mut self.config) {
            self.action_failed(error.to_string());
            return;
        }
        self.sync_keymap();
        match value {
            SettingValue::Grammar(kind) => {
                if let Ok(grammar) = ActiveGrammar::new(kind) {
                    self.grammar = grammar;
                    self.mode = self.grammar.preferred_mode().unwrap_or(Mode::Normal);
                }
            }
            SettingValue::Text(ref name) if setting == SettingId::Theme => {
                if let Ok(theme) = self.config.resolve_theme(name) {
                    self.replace_theme(name.clone(), theme);
                }
            }
            SettingValue::Boolean(_)
            | SettingValue::Integer(_)
            | SettingValue::WorkspaceMode(_)
            | SettingValue::ExplorerSort(_)
            | SettingValue::Text(_) => {}
        }
        self.status(format!(
            "previewing {}: {value} · Enter saves · Esc rolls back",
            setting.descriptor().title
        ));
    }

    pub(super) fn cancel_settings_picker(&mut self) {
        let Some(setting) = self.rollback_setting_preview() else {
            self.list = None;
            self.list_actions.clear();
            return;
        };
        self.settings_view = None;
        self.list = None;
        self.list_actions.clear();
        self.status(format!(
            "{} preview rolled back",
            setting.descriptor().title
        ));
    }

    /// Restore the runtime snapshot behind a setting preview without closing
    /// its choice list. A failed save uses this so a value that could not be
    /// persisted never remains effective merely because the person has not
    /// pressed Escape yet; the list stays open and can be retried.
    fn rollback_setting_preview(&mut self) -> Option<SettingId> {
        let SettingsView::Values(preview) = self.settings_view.as_ref()?;
        let preview = (**preview).clone();
        self.config = preview.original_config;
        self.sync_keymap();
        self.replace_theme(preview.original_theme_name, preview.original_theme);
        self.grammar = preview.original_grammar;
        self.mode = preview.original_mode;
        Some(preview.setting)
    }

    pub(super) fn persist_selected_setting(
        &mut self,
        setting: SettingId,
        value: SettingValue,
    ) -> bool {
        let Some(path) = self.config_path.clone() else {
            let recovery = self
                .rollback_setting_preview()
                .map(|_| " · preview rolled back")
                .unwrap_or("");
            self.action_failed(format!(
                "settings cannot be saved because no config path was loaded{recovery}"
            ));
            return false;
        };
        let updated = match persist_setting(&path, setting, &value) {
            Ok(updated) => updated,
            Err(error) => {
                let recovery = self
                    .rollback_setting_preview()
                    .map(|_| " · preview rolled back; Enter retries")
                    .unwrap_or("");
                self.error_from(
                    "Runyte",
                    "Settings save failed",
                    format!(
                        "could not save {}: {error}{recovery}",
                        setting.descriptor().key,
                    ),
                );
                return false;
            }
        };
        self.persisted_config = updated;
        if setting.descriptor().preview == PreviewPolicy::Immediate
            && let Err(error) = setting.apply(&value, &mut self.config)
        {
            self.rollback_setting_preview();
            self.settings_view = None;
            self.list = None;
            self.list_actions.clear();
            self.refresh_settings_buffers();
            self.error_from(
                "Runyte",
                "Settings apply failed",
                format!(
                    "saved {}, but could not apply it to this session; runtime preview rolled back, restart Runyte to apply: {error}",
                    setting.descriptor().key
                ),
            );
            return false;
        }
        self.sync_keymap();
        if setting == SettingId::NotificationsHistoryLimit {
            self.notifications
                .set_limit(self.config.notifications.history_limit);
            self.refresh_notification_buffers();
        }
        match &value {
            SettingValue::Grammar(kind) => self.select_grammar(*kind),
            SettingValue::Text(name) if setting == SettingId::Theme => {
                if let Ok(theme) = self.config.resolve_theme(name) {
                    self.replace_theme(name.clone(), theme);
                }
            }
            SettingValue::Boolean(_)
            | SettingValue::Integer(_)
            | SettingValue::WorkspaceMode(_)
            | SettingValue::ExplorerSort(_)
            | SettingValue::Text(_) => {}
        }
        // An explorer setting changed from this page has to reach the open
        // listings just as the explorer's own keys do.
        if SettingId::EXPLORER.contains(&setting)
            && let Err(error) = self.refresh_listings(None)
        {
            self.action_failed(error.to_string());
        }
        self.settings_view = None;
        self.list = None;
        self.list_actions.clear();
        self.refresh_settings_buffers();
        let suffix = if setting.descriptor().preview == PreviewPolicy::RestartRequired {
            " · restart Runyte to apply"
        } else {
            ""
        };
        self.status(format!(
            "saved {}: {value}{suffix}",
            setting.descriptor().key
        ));
        true
    }

    pub(super) fn set_theme(&mut self, name: &str) -> Result<()> {
        if let Err(error) = self.config.resolve_theme(name) {
            self.action_failed(error.to_string());
            return Ok(());
        }
        self.persist_selected_setting(SettingId::Theme, SettingValue::Text(name.to_owned()));
        Ok(())
    }

    fn replace_theme(&mut self, name: String, theme: Theme) {
        self.theme = theme;
        self.theme_name = name;
        self.sync_terminal_default_colors();
    }

    pub(super) fn sync_terminal_default_colors(&mut self) {
        self.terminals.set_default_colors(DefaultColors::new(
            self.theme.foreground.channels(),
            self.theme.background.channels(),
        ));
    }

    pub(super) fn request_quit(&mut self, force: bool, force_command: &str) {
        if self.quit_allowed(force, force_command) {
            self.quit_directory = None;
            self.persistent_exit_request = Some(super::PersistentExitRequest::Quit { force });
            self.should_quit = true;
        }
    }

    /// Leaves a persistent client without applying editor-exit guards.
    ///
    /// The host retains the complete editor state, so dirty buffers and live
    /// terminals are not being abandoned and do not require a force spelling.
    pub(super) fn request_detach(&mut self) {
        if !self.persistent_session {
            self.action_failed(":detach is available only in persistent mode");
            return;
        }
        self.tutorial_requested_detach();
        self.quit_directory = None;
        self.persistent_exit_request = Some(super::PersistentExitRequest::Detach);
        self.should_quit = true;
    }

    /// Applies Vim/Helix-style `:q` semantics to the active view.
    ///
    /// A pane's uniquely displayed buffer leaves with it, so dirty text needs
    /// the force spelling; a buffer shared by another pane stays open. A
    /// terminal session survives the pane. The last pane is the application
    /// boundary and uses the ordinary global quit guard. A commit message is a
    /// workflow rather than a document that makes sense hidden: leaving its
    /// view cancels it, with force required when authored text would be lost.
    pub(super) fn request_view_quit(&mut self, force: bool) {
        let buffer = self.active().buffer;
        if self.quit_to_covered_terminal(force) {
            return;
        }
        if self.panes.len() == 1 {
            if self.active_terminal().is_none() && self.buffers[buffer].is_commit_message() {
                if self.buffers[buffer].dirty && !force {
                    self.action_warning(
                        "Quit refused",
                        "modified commit message; use :q! to discard it and cancel the commit",
                    );
                    return;
                }
                self.abandon_commit_message(buffer);
            }
            self.request_quit(force, ":q!");
            return;
        }
        if let Some(maximized) = self.maximized {
            self.status(format!(
                "leave {} before closing the pane",
                maximized.view.label()
            ));
            return;
        }
        if self.active_terminal().is_some() {
            self.close_pane();
            return;
        }

        let displayed_elsewhere = self.panes.iter().any(|(pane_id, pane)| {
            *pane_id != self.active_pane && pane.terminal.is_none() && pane.buffer == buffer
        });
        if !displayed_elsewhere {
            if self.buffers[buffer].dirty && !force {
                self.action_warning(
                    "Quit refused",
                    "modified buffer; use :q! to discard its unsaved changes",
                );
                return;
            }
            if force {
                self.close_buffer_discarding(buffer);
            } else {
                self.close_buffer_returning_from_commit(buffer);
            }
        }
        self.close_pane();
    }

    /// Finishes with a document an external request put over a terminal.
    ///
    /// This runs before every other reading of `:q`, including the one that
    /// would stop a single-pane editor: the pane belongs to the terminal, and
    /// the document was the detour. The buffer is retired under the ordinary
    /// rules, except that a buffer another pane is also showing stays open,
    /// and then the terminal is visible again where it was.
    fn quit_to_covered_terminal(&mut self, force: bool) -> bool {
        let pane_id = self.active_pane;
        let buffer = self.active().buffer;
        if self.covered_terminal(pane_id, buffer).is_none() {
            return false;
        }
        let displayed_elsewhere = self.panes.iter().any(|(other, pane)| {
            *other != pane_id && pane.terminal.is_none() && pane.buffer == buffer
        });
        if !displayed_elsewhere {
            if self.buffers[buffer].dirty && !force {
                self.action_warning(
                    "Quit refused",
                    "modified buffer; use :q! to discard its unsaved changes",
                );
                return true;
            }
            if force {
                self.close_buffer_discarding(buffer);
            } else {
                self.close_buffer_returning_from_commit(buffer);
            }
        }
        self.uncover_terminal(pane_id, buffer);
        true
    }

    pub(super) fn request_quit_here(&mut self, force: bool) {
        if !self.quit_directory_handoff {
            self.action_failed(":qh requires the runyte() shell function from README.md");
            return;
        }
        if !self.quit_allowed(force, ":qh!") {
            return;
        }
        let requested = self.quit_here_directory();
        let directory = match fs::canonicalize(&requested) {
            Ok(directory) if directory.is_dir() => directory,
            Ok(_) => {
                self.action_failed(format!(
                    "cannot quit here: {} is not a directory",
                    requested.display()
                ));
                return;
            }
            Err(error) => {
                self.action_failed(format!(
                    "cannot quit here at {}: {error}",
                    requested.display()
                ));
                return;
            }
        };
        self.working_directory = directory.clone();
        self.quit_directory = Some(directory);
        self.persistent_exit_request = Some(super::PersistentExitRequest::Quit { force });
        self.should_quit = true;
    }

    fn quit_allowed(&mut self, force: bool, force_command: &str) -> bool {
        if !force && self.buffers.iter().any(|buffer| buffer.dirty) {
            self.action_warning(
                "Quit refused",
                format!("unsaved changes; use {force_command} to discard them"),
            );
            return false;
        }
        // A terminal can only be ended by its child or the terminal manager.
        // `:detach` bypasses this guard because it leaves the host alive, but
        // every quit spelling ends its owner in either deployment mode.
        let running = self
            .terminals
            .iter()
            .filter(|session| session.live())
            .count();
        if running > 0 {
            let plural = if running == 1 { "" } else { "s" };
            self.action_failed(format!(
                "{running} terminal{plural} still running; close {} in :terminals before quitting",
                if running == 1 { "it" } else { "them" }
            ));
            return false;
        }
        true
    }

    /// Records the loaded config without hiding unavailable editing or syntax
    /// grammars discovered during construction.
    pub fn note_loaded_config(&mut self, path: &Path) {
        self.config_path = Some(path.to_path_buf());
        self.persisted_config = self.config.clone();
        let errors = self.registry.errors();
        let configured_grammar_error = None::<&str>;
        let help = format!(
            ":? or {} for help",
            self.key_text(crate::key_spelling::actionable::HELP)
        );
        let mut parts = Vec::new();
        if let Some(error) = configured_grammar_error {
            parts.push(error.to_owned());
        }
        if !errors.is_empty() {
            parts.push(startup_status(&errors, &help));
        } else if parts.is_empty() {
            parts.push(help);
        }
        let message = format!("config: {} · {}", path.display(), parts.join(" · "));
        if errors.is_empty() && configured_grammar_error.is_none() {
            self.status(message);
        } else {
            self.error_from("Runyte", "Configuration failed", message);
        }
    }

    /// Reports each failed lazy language configuration once. A cached failure
    /// is still returned by the registry on later parses, but does not replace
    /// an unrelated success status again.
    pub(super) fn report_new_registry_errors(&mut self) -> bool {
        let unseen = self
            .registry
            .errors()
            .into_iter()
            .filter(|error| {
                self.reported_registry_errors
                    .insert((error.language, error.plain))
            })
            .collect::<Vec<_>>();
        if unseen.is_empty() {
            return false;
        }
        let summary = registry_failure_summary(&unseen);
        let message = if self.status.is_empty() {
            summary
        } else {
            format!("{} │ {summary}", self.status)
        };
        self.error_from("Runyte", "Language configuration failed", message);
        true
    }

    pub(super) fn status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_error = false;
        self.status_revision = self.status_revision.wrapping_add(1);
    }

    pub(super) fn report_completed_action(
        &mut self,
        spelling: &str,
        description: &str,
        outcome: CommandOutcome,
    ) {
        if self.fs_confirmation.is_some()
            || self.directory_reload_confirmation.is_some()
            || self.file_reload_confirmation.is_some()
            || self.buffer_discard_confirmation.is_some()
            || self.git_discard_confirmation.is_some()
            || self.git_stash_confirmation.is_some()
            || self.git_branch_switch.is_some()
            || self.git_branch_deletion.is_some()
            || self.git_pull_rebase.is_some()
            || self.git_worktree_removal.is_some()
        {
            return;
        }
        let (detail, is_error) = match outcome {
            CommandOutcome::Completed => (Some(description.to_owned()), false),
            CommandOutcome::Status(message)
            | CommandOutcome::AsynchronousRequest(Some(message)) => (Some(message), false),
            CommandOutcome::AsynchronousRequest(None) => (Some(description.to_owned()), false),
            CommandOutcome::UserError(message) => (
                Some(format!(
                    "{description}{}",
                    outcome_clause("failed", &message)
                )),
                true,
            ),
            CommandOutcome::Unavailable(message) => (
                Some(format!(
                    "{description}{}",
                    outcome_clause("unavailable", &message)
                )),
                false,
            ),
            CommandOutcome::Confirmation(_) | CommandOutcome::Prompt(_) => (None, false),
        };
        if let Some(detail) = detail {
            let id = self.active_action_id.unwrap_or_else(|| {
                let id = self.next_action_id;
                self.next_action_id = self.next_action_id.wrapping_add(1).max(1);
                id
            });
            self.action_feedback = Some(ActionFeedback {
                id,
                spelling: spelling.to_owned(),
                text: format!("{spelling} ({detail})"),
                is_error,
            });
        }
    }

    pub(super) fn mark_action_feedback_failed(&mut self, action: Option<u64>, message: &str) {
        let Some(action) = action else {
            return;
        };
        let Some(feedback) = self.action_feedback.as_mut() else {
            return;
        };
        // `is_error` is the idempotency guard: once this echo has been
        // marked failed, a second asynchronous failure for the same action
        // (which should not happen, but would otherwise double-append)
        // leaves it alone rather than matching on the suffix text itself.
        if feedback.id != action || feedback.is_error {
            return;
        }
        let suffix = outcome_clause("failed", message);
        if feedback.text.ends_with(')') {
            feedback.text.pop();
            feedback.text.push_str(&suffix);
            feedback.text.push(')');
        } else {
            feedback.text.push_str(&suffix);
        }
        feedback.is_error = true;
    }

    pub(super) fn update_action_feedback(&mut self, action: Option<u64>, detail: &str) -> bool {
        let Some(action) = action else {
            return false;
        };
        let Some(feedback) = self.action_feedback.as_mut() else {
            return false;
        };
        if feedback.id != action {
            return false;
        }
        feedback.text = format!("{} ({detail})", feedback.spelling);
        true
    }

    /// Live echo of keys typed so far that have not yet resolved to a
    /// command: a chord prefix, a numeric count, or a character-taking
    /// command awaiting its operand. `None` once nothing is pending, so
    /// [`Self::displayed_status_message`]'s completed-action text can show
    /// through instead.
    pub(crate) fn live_pending_display(&self) -> Option<String> {
        if let Some(command) = self.grammar.awaiting_character() {
            return Some(format!("{} …", command.metadata().description));
        }
        let count = self.grammar.pending_count();
        let sequence = self.grammar.pending_sequence();
        if count.is_none() && sequence.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        if let Some(count) = count {
            parts.push(count.to_string());
        }
        if !sequence.is_empty() {
            parts.push(sequence.to_string());
        }
        Some(format!("{} …", parts.join(" ")))
    }

    pub(crate) fn displayed_status_message(&self) -> &str {
        self.action_feedback
            .as_ref()
            .map_or("", |feedback| feedback.text.as_str())
    }

    /// Whether [`Self::displayed_status_message`] reports a failure, for the
    /// interaction line's error/non-error styling distinction.
    pub(crate) fn displayed_status_message_is_error(&self) -> bool {
        self.action_feedback
            .as_ref()
            .is_some_and(|feedback| feedback.is_error)
    }

    pub(super) fn mark_unavailable(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.status = message.clone();
        self.status_error = false;
        self.status_revision = self.status_revision.wrapping_add(1);
        self.unavailable_revision = self.unavailable_revision.wrapping_add(1);
        self.push_notification(NotificationDraft::new(
            NotificationSeverity::Info,
            "Runyte",
            "Action unavailable",
            message,
        ));
    }

    /// Like [`Self::mark_unavailable`], but does not retain a notification.
    ///
    /// Used when a language-server request is suppressed because the server
    /// never advertised the capability it needs: `CommandOutcome::Unavailable`
    /// and the interaction line still need to hear about it, but this is
    /// reachable once per keystroke while typing next to a trigger character
    /// the server does not support (`(` and `,` for signature help, `.` and
    /// `:` for completion), and nothing about "this server cannot do this" is
    /// worth reading back later. A `Method not found` that arrives from a
    /// server that did advertise the capability is a real protocol violation
    /// and still goes through `error`, retained as usual.
    pub(super) fn mark_unsupported(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_error = false;
        self.status_revision = self.status_revision.wrapping_add(1);
        self.unavailable_revision = self.unavailable_revision.wrapping_add(1);
    }

    /// Reports an expected refusal at the editor-action boundary.
    ///
    /// The action still failed, so the interaction line uses its failure
    /// styling and command dispatch returns `CommandOutcome::UserError`.
    /// Retained severity is independent of that outcome: current-context
    /// refusals are informational because Runyte and its dependencies are
    /// still operating normally.
    pub(super) fn action_failed(&mut self, message: impl Into<String>) {
        self.failure_from(FailureClass::Routine, "Runyte", "Action failed", message);
    }

    /// Reports a failure without retaining a notification.
    ///
    /// Used for `No binding: X`: it is already visible at the moment it
    /// happens, through the same status this sets, and through the key
    /// hints, which read the grammar notice directly rather than this
    /// status. A burst of mistyping is otherwise the single largest
    /// contributor to a notification count that never goes away, because
    /// unlike most other errors it has no natural rate limit — nothing stops
    /// the next keystroke from producing another one.
    pub(super) fn error_unretained(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_error = true;
        self.status_revision = self.status_revision.wrapping_add(1);
    }

    pub(super) fn error_from(
        &mut self,
        source: impl Into<String>,
        title: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.failure_from(FailureClass::Fault, source, title, message);
    }

    /// Reports a refusal or incomplete result that needs attention because it
    /// protects data or leaves state needing review.
    pub(super) fn action_warning(&mut self, title: impl Into<String>, message: impl Into<String>) {
        self.failure_from(FailureClass::Protective, "Runyte", title, message);
    }

    /// Maps one semantic failure to retained presentation without changing
    /// the interaction line's failed-action styling.
    pub(super) fn failure_from(
        &mut self,
        class: FailureClass,
        source: impl Into<String>,
        title: impl Into<String>,
        message: impl Into<String>,
    ) {
        let message = message.into();
        self.status.clone_from(&message);
        self.status_error = true;
        self.status_revision = self.status_revision.wrapping_add(1);
        self.push_notification(NotificationDraft::new(
            match class {
                FailureClass::Routine => NotificationSeverity::Info,
                FailureClass::Protective => NotificationSeverity::Warning,
                FailureClass::Fault => NotificationSeverity::Error,
            },
            source,
            title,
            message,
        ));
    }

    /// Like [`Self::action_warning`], but for a protective condition already
    /// retained by the producer that detected it.
    pub(super) fn action_warning_unretained(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_error = true;
        self.status_revision = self.status_revision.wrapping_add(1);
    }

    /// Reports a search that ran cleanly and simply found nothing.
    pub(super) fn search_info(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.status.clone_from(&message);
        self.status_error = false;
        self.status_revision = self.status_revision.wrapping_add(1);
        self.push_notification(NotificationDraft::new(
            NotificationSeverity::Info,
            "Runyte",
            "Search",
            message,
        ));
    }

    pub(super) fn info_from(
        &mut self,
        source: impl Into<String>,
        title: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.push_notification(NotificationDraft::new(
            NotificationSeverity::Info,
            source,
            title,
            message,
        ));
    }

    /// Reports a failure detected by a host boundary after command dispatch.
    /// Hosts enter here so it is retained without replacing the action echo.
    pub fn report_host_error(&mut self, message: impl Into<String>) {
        self.error_from("Host", "Host operation failed", message);
    }
}

/// Projects the installed logger's state onto one service-health row.
///
/// A free function over an owned status so both halves of the policy — a
/// healthy log and a degraded one — are coverable without installing a
/// process-wide logger in a test.
fn logging_health_entry(status: Option<crate::log::Status>) -> ServiceHealthEntry {
    let Some(status) = status else {
        return ServiceHealthEntry::new(
            "log",
            ServiceState::Disabled,
            "no diagnostic log is installed for this process",
        );
    };
    let destination = status.path.as_ref().map_or_else(
        || "no destination".to_owned(),
        |path| path.display().to_string(),
    );
    let level = status.level.map_or_else(
        || "not recording".to_owned(),
        |level| level.label().to_ascii_lowercase(),
    );
    match (status.failure, status.level) {
        (Some(failure), _) => ServiceHealthEntry::new(
            "log",
            ServiceState::Degraded,
            format!(
                "{} owner · {level} · {destination} · {failure}",
                status.role
            ),
        ),
        (None, Some(_)) => ServiceHealthEntry::new(
            "log",
            ServiceState::Ready,
            format!("{} owner · {level} · {destination}", status.role),
        ),
        (None, None) => ServiceHealthEntry::new(
            "log",
            ServiceState::Disabled,
            format!("{} owner · {level} · {destination}", status.role),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::logging_health_entry;
    use crate::{
        log::{Level, Role, Status},
        service_health::ServiceState,
    };
    use std::path::PathBuf;

    #[test]
    fn service_health_names_the_owner_level_and_resolved_path() {
        let entry = logging_health_entry(Some(Status {
            role: Role::Host,
            level: Some(Level::Info),
            path: Some(PathBuf::from("/work/api/.runyte/host.log")),
            failure: None,
        }));

        assert_eq!(entry.service, "log");
        assert_eq!(entry.state, ServiceState::Ready);
        assert_eq!(
            entry.detail,
            "host owner · info · /work/api/.runyte/host.log"
        );
    }

    #[test]
    fn a_logger_that_could_not_be_installed_reports_degraded_with_its_failure() {
        let entry = logging_health_entry(Some(Status {
            role: Role::Standalone,
            level: None,
            path: Some(PathBuf::from("/work/api/.runyte/standalone-7.log")),
            failure: Some("cannot open the diagnostic log: Is a directory".to_owned()),
        }));

        assert_eq!(entry.state, ServiceState::Degraded);
        assert!(
            entry.detail.contains("standalone owner"),
            "{}",
            entry.detail
        );
        assert!(entry.detail.contains("Is a directory"), "{}", entry.detail);
    }

    #[test]
    fn a_process_without_a_logger_reports_it_plainly() {
        let entry = logging_health_entry(None);
        assert_eq!(entry.state, ServiceState::Disabled);
        assert_eq!(
            entry.detail,
            "no diagnostic log is installed for this process"
        );
    }
}
