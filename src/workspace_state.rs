// SPDX-License-Identifier: MPL-2.0

//! Runtime workspace storage, independent of workspace discovery. Selecting
//! out-of-tree storage is explicit. Ordinary in-tree placement selection never
//! asks for account paths; reservation validation resolves its protected root
//! separately. The home-workspace policy is owned by project_root.

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
        let user_root = if matches!(self, Self::InTree(_)) {
            user_state_root()
        } else {
            None
        };
        self.validate_with_user_root(reserved, user_root.as_deref())
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
        self.validate(reserved)?;
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
