// SPDX-License-Identifier: MPL-2.0

//! Moving the editor between workspaces: `:cd`, `:init` and the explorer's
//! `Tab c`.
//!
//! A directory belongs to the workspace ordinary discovery finds for it. When
//! that is the workspace already open, only the working directory moves. When
//! it is another one, a persistent session attaches to it and leaves its own
//! host running, while an ide editor has no host and is replaced by a new
//! process rooted there. A workspace's identity, state location and services
//! are all fixed when it starts, so nothing here re-roots a live editor.

use super::{App, Path, PathBuf, Result, WorkspaceRelaunchRequest};
use crate::project_root;
use anyhow::{Context, bail, ensure};

/// Where a directory leads, relative to the workspace already open.
#[derive(Debug, Eq, PartialEq)]
enum Destination {
    Current,
    Other(PathBuf),
    /// Discovery found no workspace for the directory.
    Unclaimed,
    /// Discovery failed, so nothing is known about the directory.
    Undetermined(String),
}

fn canonical_directory(requested: &Path) -> Result<PathBuf> {
    let directory = std::fs::canonicalize(requested).map_err(|error| {
        anyhow::anyhow!(
            "failed to change directory to {}: {error}",
            requested.display()
        )
    })?;
    ensure!(
        directory.is_dir(),
        "cannot change directory to {}: not a directory",
        directory.display()
    );
    Ok(directory)
}

impl App {
    pub(super) fn change_directory(&mut self, requested: Option<PathBuf>) -> Result<()> {
        let directory = match requested {
            Some(path) => self.resolve_working_path(path),
            None => self.contextual_directory().context(
                "`:cd` needs a directory here; name one, or run it from an explorer or a terminal",
            )?,
        };
        let directory = canonical_directory(&directory)?;
        if self.editor_mode {
            return self.move_working_directory(directory);
        }
        match self.destination_of(&directory) {
            // A directory that cannot be classified is still somewhere to stand.
            Destination::Current | Destination::Undetermined(_) => {
                self.move_working_directory(directory)
            }
            Destination::Other(root) => {
                self.enter_workspace(root, directory);
                Ok(())
            }
            Destination::Unclaimed => {
                self.move_working_directory(directory.clone())?;
                // A dirty explorer leaves its own question in the status line.
                if self.directory_reload_confirmation.is_none() {
                    self.status(format!(
                        "working directory: {} · no workspace of its own; :init makes one",
                        directory.display()
                    ));
                }
                Ok(())
            }
        }
    }

    /// `:init [DIRECTORY]`: the exact directory becomes a workspace, as with
    /// `runyte --init`, and the editor moves there.
    pub(super) fn initialize_workspace(&mut self, requested: Option<PathBuf>) -> Result<()> {
        let directory = match requested {
            Some(path) => self.resolve_working_path(path),
            None => self
                .contextual_directory()
                .unwrap_or_else(|| self.working_directory.clone()),
        };
        let directory = canonical_directory(&directory)?;
        if directory == self.project_root {
            self.status(format!("{} is already a workspace", directory.display()));
            return Ok(());
        }
        if self.refuse_workspace_change(&directory) || self.refuse_destination(&directory) {
            return Ok(());
        }
        let root = project_root::initialize_with_safety(
            &directory,
            &self.config.workspace.state,
            &self.workspace_change_safety(),
        )?;
        self.enter_workspace(root, directory);
        Ok(())
    }

    /// The explorer's `Tab c`. It goes where `:cd` would, and pointing at a
    /// directory is deliberate enough to also make it a workspace when
    /// discovery finds none. Moving around with `:cd` never creates state.
    pub(super) fn change_to_explorer_workspace(&mut self) {
        let Some(path) = self
            .active_buffer()
            .is_directory()
            .then(|| self.active_buffer().path.clone())
            .flatten()
        else {
            self.action_failed("active buffer is not an explorer");
            return;
        };
        let directory = match canonical_directory(&path) {
            Ok(directory) => directory,
            Err(error) => {
                self.action_failed(format!("{error:#}"));
                return;
            }
        };
        match self.destination_of(&directory) {
            Destination::Current => {
                self.status(format!("working directory: {}", directory.display()));
                self.working_directory = directory;
            }
            Destination::Other(root) => self.enter_workspace(root, directory),
            Destination::Undetermined(reason) => {
                self.action_failed(format!(
                    "cannot tell which workspace {} belongs to: {reason}",
                    directory.display()
                ));
            }
            Destination::Unclaimed => {
                if self.refuse_workspace_change(&directory) || self.refuse_destination(&directory) {
                    return;
                }
                match self.adopt_directory(&directory) {
                    Ok(root) => self.enter_workspace(root, directory),
                    Err(error) => self.action_failed(format!("{error:#}")),
                }
            }
        }
    }

    /// Applies the directory a `:cd` in another persistent session named, once
    /// the client has attached here.
    pub fn arrive_in_directory(&mut self, directory: &Path) -> Result<()> {
        let directory = canonical_directory(directory)?;
        ensure!(
            directory.starts_with(&self.project_root),
            "{} is outside workspace {}",
            directory.display(),
            self.project_root.display()
        );
        self.working_directory = directory;
        Ok(())
    }

    pub fn take_workspace_relaunch(&mut self) -> Option<WorkspaceRelaunchRequest> {
        self.workspace_relaunch.take()
    }

    /// The directory the active view is showing, when it is showing one.
    pub(super) fn contextual_directory(&self) -> Option<PathBuf> {
        if let Some(terminal) = self.active_terminal() {
            return self
                .terminals
                .get(terminal)
                .map(|session| session.directory().to_path_buf());
        }
        let buffer = self.active_buffer();
        buffer.is_directory().then(|| buffer.path.clone()).flatten()
    }

    fn destination_of(&self, directory: &Path) -> Destination {
        let found = match project_root::discover_with_home(directory, &self.config.workspace.state)
        {
            Ok(found) => found,
            Err(error) => return Destination::Undetermined(error.to_string()),
        };
        let inside = directory.starts_with(&self.project_root);
        match found {
            // A home workspace owns no subdirectory by discovery, and adopting
            // one with `Tab c` is how a project starts there. Any other root
            // discovery cannot find, such as one with an absolute state
            // directory, still contains what is below it.
            None if directory == self.project_root || (inside && !self.home_workspace) => {
                Destination::Current
            }
            None => Destination::Unclaimed,
            // A workspace deliberately made inside a repository stays open
            // while moving around in it, though discovery names the repository.
            Some(found)
                if found.root == self.project_root
                    || (inside && self.project_root.starts_with(&found.root)) =>
            {
                Destination::Current
            }
            Some(found) => Destination::Other(found.root),
        }
    }

    fn enter_workspace(&mut self, root: PathBuf, directory: PathBuf) {
        if self.persistent_session {
            if self.request_workspace_switch_for_platform(root, cfg!(any(unix, windows))) {
                if let Some(switch) = self.workspace_switch.as_mut() {
                    switch.working_directory = directory;
                    switch.arrive_in_working_directory = true;
                }
                if cfg!(unix) {
                    self.should_quit = true;
                }
            }
            return;
        }
        if self.refuse_workspace_change(&directory) || self.refuse_destination(&root) {
            return;
        }
        #[cfg(windows)]
        let directory = match crate::windows_fs::ordinary_working_directory(&directory) {
            Ok(directory) => directory,
            Err(error) => {
                self.action_failed(format!("cannot open {}: {error}", directory.display()));
                return;
            }
        };
        self.quit_directory = None;
        self.workspace_relaunch = Some(WorkspaceRelaunchRequest { root, directory });
        self.should_quit = true;
    }

    /// Leaving an ide workspace ends everything in this process, so what
    /// `:quit` would protect blocks the move. A persistent session keeps its
    /// work in the host it leaves behind, and nothing blocks it.
    fn refuse_workspace_change(&mut self, directory: &Path) -> bool {
        let Some(reason) = (!self.persistent_session)
            .then(|| self.workspace_change_obstacle(directory))
            .flatten()
        else {
            return false;
        };
        self.action_warning("Workspace change refused", reason);
        true
    }

    #[cfg_attr(not(windows), allow(unused_variables))]
    fn workspace_change_obstacle(&self, directory: &Path) -> Option<String> {
        #[cfg(windows)]
        if let Err(error) = crate::windows_fs::ordinary_working_directory(directory) {
            return Some(format!("{}: {error}", directory.display()));
        }
        // Replacing the process needs its program to still be there. An
        // upgrade or uninstall during the session would otherwise end the
        // editor and only then fail to start its successor.
        match std::env::current_exe() {
            Ok(executable) if executable.is_file() => {}
            _ => {
                return Some(
                    "the Runyte program is no longer available; restart Runyte to change workspace"
                        .to_owned(),
                );
            }
        }
        if self.plugins.filesystem_applying || !self.plugins.document_saves.is_empty() {
            return Some("filesystem writes are still pending; wait for completion".to_owned());
        }
        let leases = self.plugin_activity_count();
        let jobs = self.plugin_active_job_count();
        if leases > 0 || jobs > 0 {
            return Some(format!(
                "{leases} activity leases and {jobs} plugin jobs; cancel work or stop its owner in :plugins"
            ));
        }
        if self.buffers.iter().any(|buffer| buffer.dirty) {
            return Some("unsaved changes; save or discard them first".to_owned());
        }
        let running = self
            .terminals
            .iter()
            .filter(|session| session.live())
            .count();
        (running > 0).then(|| {
            let (plural, pronoun) = if running == 1 {
                ("", "it")
            } else {
                ("s", "them")
            };
            format!("{running} terminal{plural} still running; close {pronoun} in :terminals first")
        })
    }

    /// Everything a successor would refuse has to be refused while there is
    /// still an editor to report it. A persistent switch fails safely in its
    /// client and keeps the session it came from.
    fn refuse_destination(&mut self, root: &Path) -> bool {
        if self.persistent_session {
            return false;
        }
        let Err(error) = self.validate_destination(root) else {
            return false;
        };
        self.action_failed(format!(
            "cannot open the workspace at {}: {error:#}",
            root.display()
        ));
        true
    }

    /// Runs the checks a launch rooted at `root` would make. The successor
    /// reads the configuration file again, so that file is what is judged
    /// rather than the copy loaded when this editor started.
    fn validate_destination(&self, root: &Path) -> Result<()> {
        let state = match self.config_path.as_deref() {
            Some(path) => crate::config::Config::load(Some(path))?.0.workspace.state,
            None => self.config.workspace.state.clone(),
        };
        let safety = self.workspace_change_safety();
        let location = project_root::resolve_state_location_with_safety(root, &state, &safety)?;
        safety.validate(root, &location)?;
        project_root::exact_origin_with_safety(root, &state, &safety)?;
        Ok(())
    }

    fn adopt_directory(&self, directory: &Path) -> Result<PathBuf> {
        let safety = self.workspace_change_safety();
        let state = &self.config.workspace.state;
        if let Some(reason) = project_root::automatic_workspace_refusal(
            directory,
            directory,
            state,
            &safety,
            &project_root::user_home_directories(),
            &[],
        )? {
            bail!("cannot make {} a workspace: {reason}", directory.display());
        }
        project_root::initialize_with_safety(directory, state, &safety)
    }

    fn workspace_change_safety(&self) -> crate::workspace_state::StartupSafety {
        let reserved = [
            self.config_path
                .as_deref()
                .map(|path| crate::config::config_root_for(path, &self.working_directory))
                .or_else(crate::config::default_config_root),
            crate::external_open::cache_root(),
        ];
        crate::workspace_state::StartupSafety::capture(
            &reserved.into_iter().flatten().collect::<Vec<_>>(),
        )
    }
}
