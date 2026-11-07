// SPDX-License-Identifier: MPL-2.0

//! Runtime workspace storage, independent of workspace discovery. Selecting
//! out-of-tree storage is explicit. Startup captures account and home paths
//! once for placement and reservation validation. The home-workspace policy
//! is owned by project_root.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The selected runtime storage for one workspace. Consumers use the path;
/// endpoint placement and durability retain the placement discriminator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateLocation {
    InTree(PathBuf),
    OutOfTree(PathBuf),
}

impl StateLocation {
    pub fn in_tree(project: &Path, configured: &Path) -> Self {
        Self::InTree(if configured.is_absolute() {
            configured.to_owned()
        } else {
            project.join(configured)
        })
    }

    /// `user_root` is a captured per-user Runyte state root, not its parent.
    pub fn out_of_tree(project: &Path, user_root: &Path) -> Self {
        Self::OutOfTree(
            user_root
                .join("workspaces")
                .join(crate::workspace::workspace_id(project)),
        )
    }

    pub fn as_path(&self) -> &Path {
        match self {
            Self::InTree(path) | Self::OutOfTree(path) => path,
        }
    }

    pub fn into_path_buf(self) -> PathBuf {
        match self {
            Self::InTree(path) | Self::OutOfTree(path) => path,
        }
    }

    /// Captures the user root only when the caller selected out-of-tree state.
    pub fn resolve_with(
        project: &Path,
        configured: &Path,
        out_of_tree: bool,
        user_root: impl FnOnce() -> Result<PathBuf>,
    ) -> Result<Self> {
        if out_of_tree {
            Ok(Self::out_of_tree(project, &user_root()?))
        } else {
            Ok(Self::in_tree(project, configured))
        }
    }

    pub fn validate(&self, reserved: &[PathBuf]) -> Result<()> {
        StartupSafety::capture_for_location(reserved, self).validate(self.as_path(), self)
    }

    /// Injected reservation roots keep storage tests independent of the account.
    pub fn validate_with_user_root(
        &self,
        reserved: &[PathBuf],
        user_root: Option<&Path>,
    ) -> Result<()> {
        self.validate_with_platform(reserved, user_root, StatePlatform::CURRENT)
    }

    pub fn validate_with_platform(
        &self,
        reserved: &[PathBuf],
        user_root: Option<&Path>,
        platform: StatePlatform,
    ) -> Result<()> {
        match self {
            Self::InTree(path) => {
                crate::project_root::validate_state_root(path, reserved)?;
                if let Some(root) = user_root {
                    crate::project_root::validate_state_root(path, &[root.to_owned()])?;
                }
                Ok(())
            }
            Self::OutOfTree(path) => {
                let subtree = path.parent().context("workspace state has no parent")?;
                let boundary = if matches!(platform, StatePlatform::Windows) {
                    subtree
                } else {
                    subtree
                        .parent()
                        .context("workspace state has no user root")?
                };
                crate::project_root::validate_state_root(boundary, reserved)
            }
        }
    }

    /// Creates the selected private subtree, after checking reserved storage.
    /// Windows intentionally shares its application container with cache;
    /// workspace data occupies the separate `workspaces` sibling.
    pub fn prepare(&self, reserved: &[PathBuf]) -> Result<()> {
        let safety = StartupSafety::capture_for_location(reserved, self);
        self.prepare_with_safety(self.as_path(), &safety)
    }

    pub fn prepare_with_safety(&self, project: &Path, safety: &StartupSafety) -> Result<()> {
        safety.validate(project, self)?;
        match self {
            Self::InTree(path) => {
                crate::private_storage::Directory::open(path, true)?;
            }
            Self::OutOfTree(path) => {
                let subtree = path.parent().context("workspace state has no parent")?;
                let root = subtree
                    .parent()
                    .context("workspace state has no user root")?;
                crate::private_storage::Directory::open_private_checked(root)?;
                crate::private_storage::Directory::open_private_checked(subtree)?;
                crate::private_storage::Directory::open_private_checked(path)?;
            }
        }
        Ok(())
    }

    /// Out-of-tree plugin state is durable beneath the captured application
    /// container. An explicit compatible Windows anchor remains authoritative.
    pub fn plugin_anchor(&self, configured: Option<PathBuf>) -> Option<PathBuf> {
        configured.or_else(|| match self {
            Self::OutOfTree(path) => path.parent()?.parent()?.parent().map(Path::to_owned),
            Self::InTree(_) => None,
        })
    }
}

/// A single startup's frozen, alias-safe user-storage reservations. The CLI
/// passes this admission to App through Config, which consumes and closes it.
/// A cloned carrier cannot authorize a later startup without fresh capture.
#[derive(Debug)]
pub struct StartupSafety {
    reserved: Vec<(PathBuf, PathBuf)>,
    user_root: Option<(PathBuf, PathBuf)>,
    user_root_source: UserRootSource,
    homes: Vec<(PathBuf, PathBuf)>,
    #[cfg(any(unix, windows))]
    context_root: Option<PathBuf>,
    #[cfg(any(unix, windows))]
    context_standard: bool,
    #[cfg(windows)]
    context_location: Option<crate::workspace::context::storage::StorageLocation>,
    admission: std::sync::Mutex<Admission>,
    #[cfg(test)]
    normalizations: std::sync::atomic::AtomicUsize,
}

#[derive(Clone, Copy, Debug)]
enum UserRootSource {
    Account,
    Explicit,
}

/// Keeps a constructor's captured admission scoped even when an earlier step
/// fails before storage admission. Nested constructor guards share ownership.
pub(crate) struct StartupSafetyGuard(Option<std::sync::Arc<StartupSafety>>);

impl Drop for StartupSafetyGuard {
    fn drop(&mut self) {
        if let Some(safety) = &self.0 {
            safety.close();
        }
    }
}

#[derive(Debug, Default)]
struct Admission {
    closed: bool,
    selected: Option<(PathBuf, StateLocation)>,
}

impl StartupSafety {
    pub(crate) fn close_on_exit(captured: Option<&std::sync::Arc<Self>>) -> StartupSafetyGuard {
        StartupSafetyGuard(captured.cloned())
    }

    pub fn capture(reserved: &[PathBuf]) -> Self {
        let mut safety = Self {
            user_root_source: UserRootSource::Account,
            ..Self::capture_with_homes(
                reserved,
                user_state_root,
                &crate::project_root::user_home_directories(),
            )
        };
        #[cfg(unix)]
        {
            safety.context_root = crate::workspace::context::storage::Storage::default_root();
        }
        #[cfg(windows)]
        {
            safety.context_location =
                crate::workspace::context::storage::Storage::default_location();
            safety.context_root = safety
                .context_location
                .as_ref()
                .map(|location| location.root().to_owned());
        }
        #[cfg(any(unix, windows))]
        {
            safety.context_standard = std::env::var_os("RUNYTE_CONTEXT_HOME").is_none();
        }
        safety
    }

    fn capture_for_location(reserved: &[PathBuf], location: &StateLocation) -> Self {
        match location {
            StateLocation::InTree(_) => Self::capture(reserved),
            StateLocation::OutOfTree(_) => Self::capture_with(reserved, || None),
        }
    }

    /// Injects a fixed raw root source. A later startup re-canonicalizes those
    /// paths instead of falling back to the account/environment resolver.
    pub fn capture_with(
        reserved: &[PathBuf],
        resolve_user_root: impl FnOnce() -> Option<PathBuf>,
    ) -> Self {
        Self::capture_with_homes(reserved, resolve_user_root, &[])
    }

    /// Injects exact home identities as well as the state root. No account paths
    /// are consulted, including when this capture is refreshed.
    pub fn capture_with_homes(
        reserved: &[PathBuf],
        resolve_user_root: impl FnOnce() -> Option<PathBuf>,
        homes: &[PathBuf],
    ) -> Self {
        let user_root = resolve_user_root();
        let homes = homes
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter_map(|home| {
                home.canonicalize()
                    .ok()
                    .map(|canonical| (home.clone(), canonical))
            })
            .collect::<Vec<_>>();
        #[cfg(test)]
        let count = reserved.len() + usize::from(user_root.is_some()) + homes.len();
        Self {
            reserved: reserved
                .iter()
                .map(|path| (path.clone(), crate::project_root::comparable_path(path)))
                .collect(),
            user_root: user_root.map(|path| {
                let normalized = crate::project_root::comparable_path(&path);
                (path, normalized)
            }),
            user_root_source: UserRootSource::Explicit,
            homes,
            #[cfg(any(unix, windows))]
            context_root: None,
            #[cfg(any(unix, windows))]
            context_standard: false,
            #[cfg(windows)]
            context_location: None,
            admission: std::sync::Mutex::new(Admission::default()),
            #[cfg(test)]
            normalizations: std::sync::atomic::AtomicUsize::new(count),
        }
    }

    #[cfg(all(test, any(unix, windows)))]
    pub(crate) fn with_context_store(mut self, root: PathBuf, standard: bool) -> Self {
        #[cfg(windows)]
        {
            self.context_location =
                crate::workspace::context::storage::StorageLocation::explicit(root.clone()).ok();
        }
        self.context_root = Some(root);
        self.context_standard = standard;
        self
    }

    pub fn for_startup(
        captured: Option<std::sync::Arc<Self>>,
        reserved: impl FnOnce() -> Vec<PathBuf>,
    ) -> std::sync::Arc<Self> {
        if let Some(captured) = captured {
            if !captured.admission.lock().unwrap().closed {
                return captured;
            }
            let roots = captured
                .reserved
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            let mut refreshed = match captured.user_root_source {
                UserRootSource::Account => Self::capture(&roots),
                UserRootSource::Explicit => Self::capture_with_homes(
                    &roots,
                    || captured.user_root.as_ref().map(|(path, _)| path.clone()),
                    &captured
                        .homes
                        .iter()
                        .map(|(raw, _)| raw.clone())
                        .collect::<Vec<_>>(),
                ),
            };
            if matches!(captured.user_root_source, UserRootSource::Explicit) {
                #[cfg(any(unix, windows))]
                {
                    refreshed.context_root = captured.context_root.clone();
                    refreshed.context_standard = captured.context_standard;
                }
                #[cfg(windows)]
                {
                    refreshed.context_location = captured.context_location.clone();
                }
            }
            return std::sync::Arc::new(refreshed);
        }
        std::sync::Arc::new(Self::capture(&reserved()))
    }

    pub(crate) fn is_home(&self, project: &Path) -> bool {
        self.homes.iter().any(|(_, home)| home == project)
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn context_policy(
        &self,
        project: &Path,
    ) -> Option<crate::workspace::context::storage::ContextStorePolicy> {
        let root = self.context_root.clone()?;
        let resolved = crate::project_root::comparable_path(&root);
        let homes = self
            .homes
            .iter()
            .filter(|(_, home)| home == project)
            .map(|(_, home)| home.clone())
            .collect::<Vec<_>>();
        Some(
            crate::workspace::context::storage::ContextStorePolicy::captured_canonical(
                root,
                resolved,
                &homes,
                self.context_standard,
            ),
        )
    }

    #[cfg(windows)]
    pub(crate) fn context_location(
        &self,
    ) -> Option<crate::workspace::context::storage::StorageLocation> {
        self.context_location.clone()
    }

    pub(crate) fn state_root(&self) -> Option<&Path> {
        self.user_root.as_ref().map(|(raw, _)| raw.as_path())
    }

    pub fn validate(&self, project: &Path, location: &StateLocation) -> Result<()> {
        let mut admission = self.admission.lock().unwrap();
        anyhow::ensure!(
            !admission.closed,
            "startup storage safety has been consumed; capture fresh reservations"
        );
        if admission
            .selected
            .as_ref()
            .is_some_and(|(root, selected)| root == project && selected == location)
        {
            return Ok(());
        }
        let boundary = match location {
            StateLocation::InTree(path) => path.as_path(),
            StateLocation::OutOfTree(path) => {
                let subtree = path.parent().context("workspace state has no parent")?;
                if cfg!(windows) {
                    subtree
                } else {
                    subtree
                        .parent()
                        .context("workspace state has no user root")?
                }
            }
        };
        #[cfg(test)]
        self.normalizations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let normalized = crate::project_root::comparable_path(boundary);
        let roots = self.reserved.iter().chain(
            self.user_root
                .iter()
                .filter(|_| matches!(location, StateLocation::InTree(_))),
        );
        if let Some((path, _)) = roots.into_iter().find(|(_, reserved)| {
            crate::project_root::comparable_paths_overlap(&normalized, reserved)
        }) {
            anyhow::bail!(
                "{} overlaps per-user Runyte storage at {} and cannot store project data",
                boundary.display(),
                path.display()
            );
        }
        admission.selected = Some((project.to_owned(), location.clone()));
        Ok(())
    }

    pub fn close(&self) {
        let mut admission = self.admission.lock().unwrap();
        admission.closed = true;
        admission.selected = None;
    }
}

/// Pure platform mapping; tests supply account paths without consulting the OS.
#[derive(Clone, Copy, Debug)]
pub enum StatePlatform {
    Unix,
    MacOs,
    Windows,
}

impl StatePlatform {
    #[cfg(target_os = "macos")]
    const CURRENT: Self = Self::MacOs;
    #[cfg(windows)]
    const CURRENT: Self = Self::Windows;
    #[cfg(not(any(target_os = "macos", windows)))]
    const CURRENT: Self = Self::Unix;
}

pub fn root_for(
    platform: StatePlatform,
    xdg_state_home: Option<&Path>,
    account_home: Option<&Path>,
    local_app_data: Option<&Path>,
) -> Option<PathBuf> {
    let account_home = account_home.filter(|path| path.is_absolute());
    let local_app_data = local_app_data.filter(|path| path.is_absolute());
    match platform {
        StatePlatform::Unix => xdg_state_home
            .filter(|path| path.is_absolute())
            .map(|path| path.join("runyte"))
            .or_else(|| account_home.map(|home| home.join(".local/state/runyte"))),
        StatePlatform::MacOs => {
            account_home.map(|home| home.join("Library/Application Support/runyte"))
        }
        StatePlatform::Windows => local_app_data.map(|local| local.join("runyte")),
    }
}

/// Resolves paths only, without creating or canonicalizing storage. The account
/// lookup is deferred until requested. In-tree placement does not request it;
/// reservation validation does, to protect per-user storage from in-tree state.
pub fn user_state_root() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(root) = fixture_state_root() {
        return Some(root);
    }
    if cfg!(test) {
        return None;
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    if let Some(root) = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return Some(root.join("runyte"));
    }
    #[cfg(windows)]
    {
        root_for(
            StatePlatform::Windows,
            None,
            None,
            crate::user_paths::system_local_app_data_directory().as_deref(),
        )
    }
    #[cfg(unix)]
    {
        let home = crate::user_paths::system_home_directory()?;
        let platform = if cfg!(target_os = "macos") {
            StatePlatform::MacOs
        } else {
            StatePlatform::Unix
        };
        root_for(platform, None, Some(&home), None)
    }
    #[cfg(not(any(unix, windows)))]
    None
}

/// Debug subprocess fixtures override the final Runyte root on every platform.
/// Release builds do not consult this selector. Unlike XDG_STATE_HOME, this
/// deliberately works on macOS and Windows as well.
#[cfg(debug_assertions)]
pub(crate) fn fixture_state_root() -> Option<PathBuf> {
    std::env::var_os("RUNYTE_TEST_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

#[cfg(test)]
#[path = "workspace_state/tests.rs"]
mod tests;
