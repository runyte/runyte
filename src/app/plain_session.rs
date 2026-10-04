// SPDX-License-Identifier: MPL-2.0

//! Standalone sessions without a workspace.
//!
//! A plain session edits files and browses directories with no project root
//! to scope anything to. It never creates the state directory, and the
//! services that assume a project — Git, language servers, plugins, the agent
//! context endpoint and persistent sessions — are not started at all. The
//! commands that need them refuse with one reason rather than each failing in
//! its own way. Project-wide search falls back to the directory in front of
//! the reader, bounded so that it cannot walk the whole machine.

use std::path::PathBuf;

use super::App;
use crate::{
    command::{CommandCapability, CommandId},
    file_picker::ScanScope,
    service_health::{CommandAvailability, PLAIN_SESSION_REASON},
};

impl App {
    /// Turns this session into a plain one. Production startup calls it
    /// before any service is attached, so nothing workspace-scoped has run.
    pub fn enter_plain_session(&mut self) {
        self.plain = true;
    }

    pub fn is_plain(&self) -> bool {
        self.plain
    }

    /// Records whether the editor process runs as root. Injected rather than
    /// read here so tests can cover both answers.
    pub fn note_running_as_root(&mut self, running_as_root: bool) {
        self.running_as_root = running_as_root;
    }

    pub fn running_as_root(&self) -> bool {
        self.running_as_root
    }

    /// Replaces the per-user storage a workspace state directory must not
    /// overlap with the list startup validated against. Construction starts
    /// from the default configuration directory and the cache, which is
    /// wrong when `--config` names a file elsewhere.
    pub fn note_reserved_user_roots(&mut self, roots: Vec<PathBuf>) {
        self.reserved_user_roots = roots;
    }

    pub fn reserved_user_roots(&self) -> &[PathBuf] {
        &self.reserved_user_roots
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

    /// Whether `:workspace-init` has created a workspace whose services the
    /// event loop must now start. Reading it clears it.
    pub fn take_workspace_services_request(&mut self) -> bool {
        std::mem::take(&mut self.workspace_services_requested)
    }

    /// The availability a workspace-scoped capability has in this session, or
    /// `None` when the session has a workspace and the ordinary answer holds.
    pub(super) fn plain_capability(&self) -> Option<CommandAvailability> {
        self.plain
            .then(|| CommandAvailability::Unavailable(PLAIN_SESSION_REASON.to_owned()))
    }

    /// Why `id` cannot run in this session, when it is plain and the command
    /// needs a workspace. Read from the same capabilities that grey commands
    /// out in help, hints and the palette, so the two cannot disagree.
    pub(super) fn plain_session_refusal(&self, id: CommandId) -> Option<&'static str> {
        if !self.plain {
            return None;
        }
        let needs_workspace = matches!(id, CommandId::Plugin(_))
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
                )
            );
        needs_workspace.then_some(PLAIN_SESSION_REASON)
    }

    /// Refuses `id` with the plain-session reason when it needs a workspace.
    /// Returns whether it was refused.
    pub(super) fn refuse_in_plain_session(&mut self, id: CommandId) -> bool {
        let Some(reason) = self.plain_session_refusal(id) else {
            return false;
        };
        self.mark_unavailable(reason);
        true
    }

    /// The directory project-wide search covers: the project root, or in a
    /// plain session the directory the active buffer or explorer shows.
    pub(crate) fn search_root(&self) -> PathBuf {
        if self.plain {
            self.active_directory()
        } else {
            self.project_root.clone()
        }
    }

    /// The ignore-aware scope a scan rooted at `root` uses. A plain session
    /// starts scans from arbitrary system directories, so its scans are
    /// contained: one filesystem, a stricter entry cap, and no walk from `/`
    /// or a virtual filesystem.
    pub(super) fn ignoring_scan_scope(&self, root: PathBuf) -> ScanScope {
        if self.plain {
            ScanScope::contained(Some(root))
        } else {
            ScanScope::ignoring(&self.project_root)
        }
    }

    /// The scope for a scan that consults no ignore file.
    pub(super) fn unfiltered_scan_scope(&self) -> ScanScope {
        if self.plain {
            ScanScope::contained(None)
        } else {
            ScanScope::Everything
        }
    }

    /// `:workspace-init [directory]`: makes the directory this session's
    /// workspace. Like `--init`, the directory named is the boundary exactly;
    /// no ancestor is consulted. The session stays standalone, and the
    /// services a workspace has are started by the event loop once it sees
    /// the request.
    pub(super) fn initialize_workspace(&mut self, requested: Option<PathBuf>) {
        if !self.plain {
            self.action_failed(format!(
                "this session already has a workspace at {}",
                self.project_root.display()
            ));
            return;
        }
        let requested = requested.map_or_else(
            || self.active_directory(),
            |path| self.resolve_working_path(path),
        );
        let project_root = match crate::project_root::initialize(
            &requested,
            &self.config.workspace.state,
            &self.reserved_user_roots,
        ) {
            Ok(root) => root,
            Err(error) => {
                self.error_from(
                    "Workspace",
                    "Cannot create the workspace",
                    format!("{error:#}"),
                );
                return;
            }
        };
        self.adopt_workspace(project_root);
    }

    /// Gives this session the workspace at `project_root`, whose state
    /// directory already exists.
    pub(super) fn adopt_workspace(&mut self, project_root: PathBuf) {
        self.state_root =
            crate::project_root::resolve_state_root(&project_root, &self.config.workspace.state);
        if !self.directory_tree.visible {
            self.directory_tree = crate::directory_tree::DirectoryTree::new(project_root.clone());
        }
        // As `--init` enters the directory it initialized. Terminals and
        // relative paths of a workspace session start from the working
        // directory, which must not be left behind at the launch directory.
        self.working_directory = project_root.clone();
        self.project_root = project_root;
        self.plain = false;
        self.workspace_services_requested = true;
        self.status(format!(
            "workspace created at {}; starting its services",
            self.project_root.display()
        ));
    }
}
