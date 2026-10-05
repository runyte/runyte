// SPDX-License-Identifier: MPL-2.0

//! Editor mode: a standalone session with no workspace.
//!
//! Editor mode edits files and browses directories with no project root to
//! scope anything to. It never creates the state directory, and the services
//! that assume a project — Git, language servers, plugins, the agent context
//! endpoint and persistent sessions — are not started at all; nor are
//! integrated terminals. The commands that need any of them refuse with one
//! reason rather than each failing in its own way. A session is in editor
//! mode for its whole life: nothing here turns it into a workspace session.
//! Project-wide search falls back to the directory in front of the reader,
//! bounded so that it cannot walk the whole machine.

use std::path::PathBuf;

use super::App;
use crate::{
    command::{CommandCapability, CommandId},
    file_picker::ScanScope,
    service_health::{CommandAvailability, EDITOR_MODE_REASON},
};

impl App {
    /// Puts this session in editor mode. Production startup builds the
    /// editor in it; tests call this before attaching any service.
    pub fn enter_editor_mode(&mut self) {
        self.editor_mode = true;
    }

    pub fn is_editor_mode(&self) -> bool {
        self.editor_mode
    }

    /// Reports each deprecated spelling `config` still uses, naming what
    /// replaces it. Called for the configuration a process starts with and
    /// for each one `:config-reload` reads.
    pub fn note_config_deprecations(&mut self, config: &crate::config::Config) {
        for warning in config.deprecation_warnings() {
            self.push_notification(crate::notification::NotificationDraft::new(
                crate::notification::NotificationSeverity::Warning,
                "Configuration",
                "Deprecated setting",
                warning,
            ));
        }
    }

    /// Records whether the editor process runs as root. Injected rather than
    /// read here so tests can cover both answers.
    pub fn note_running_as_root(&mut self, running_as_root: bool) {
        self.running_as_root = running_as_root;
    }

    pub fn running_as_root(&self) -> bool {
        self.running_as_root
    }

    /// Whether Git, the language-server manager, and the session catalog
    /// are attached, in that order.
    pub(crate) fn workspace_ports_attached(&self) -> (bool, bool, bool) {
        (
            self.ports.git_service.is_some(),
            self.ports.has_lsp(),
            self.ports.workspace_service.is_some(),
        )
    }

    /// The availability a capability editor mode does not offer has in this
    /// session, or `None` outside editor mode, where the ordinary answer holds.
    pub(super) fn editor_mode_capability(&self) -> Option<CommandAvailability> {
        self.editor_mode
            .then(|| CommandAvailability::Unavailable(EDITOR_MODE_REASON.to_owned()))
    }

    /// Why `id` cannot run in this session, when it is in editor mode and the
    /// command needs something editor mode does not offer. Read from the same
    /// capabilities that grey commands out in help, hints and the palette, so
    /// the two cannot disagree.
    pub(super) fn editor_mode_refusal(&self, id: CommandId) -> Option<&'static str> {
        if !self.editor_mode {
            return None;
        }
        let unavailable = matches!(id, CommandId::Plugin(_))
            || matches!(
                id.capability(),
                Some(
                    CommandCapability::LspDocument
                        | CommandCapability::LspManager
                        | CommandCapability::GitProject
                        | CommandCapability::GitFetchBranch
                        | CommandCapability::GitMergeActive
                        | CommandCapability::GitMergeContinue
                        | CommandCapability::GitConflict
                        | CommandCapability::GitConflictWhole
                        | CommandCapability::GitRefresh
                        | CommandCapability::PersistentSession
                        | CommandCapability::SessionControls
                        | CommandCapability::Workspace
                        | CommandCapability::Terminals
                )
            );
        unavailable.then_some(EDITOR_MODE_REASON)
    }

    /// Refuses `id` with the editor-mode reason when editor mode does not
    /// offer it. Returns whether it was refused.
    pub(super) fn refuse_in_editor_mode(&mut self, id: CommandId) -> bool {
        let Some(reason) = self.editor_mode_refusal(id) else {
            return false;
        };
        self.mark_unavailable(reason);
        true
    }

    /// The directory project-wide search covers: the project root, or in
    /// editor mode the directory the active buffer or explorer shows.
    pub(crate) fn search_root(&self) -> PathBuf {
        if self.editor_mode {
            self.active_directory()
        } else {
            self.project_root.clone()
        }
    }

    /// The ignore-aware scope a scan rooted at `root` uses. Editor mode
    /// starts scans from arbitrary system directories, so its scans are
    /// contained: one filesystem, a stricter entry cap, and no walk from `/`
    /// or a virtual filesystem.
    pub(super) fn ignoring_scan_scope(&self, root: PathBuf) -> ScanScope {
        if self.editor_mode {
            ScanScope::contained(Some(root))
        } else {
            ScanScope::ignoring(&self.project_root)
        }
    }

    /// The scope for a scan that consults no ignore file.
    pub(super) fn unfiltered_scan_scope(&self) -> ScanScope {
        if self.editor_mode {
            ScanScope::contained(None)
        } else {
            ScanScope::Everything
        }
    }
}
