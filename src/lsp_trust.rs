// SPDX-License-Identifier: MPL-2.0

//! Per-user approval to run language servers in one exact workspace.
//! Ordinary projects cannot supply their own approval records. The account's
//! home workspace may use its standard private per-user cache.

use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub struct TrustStore {
    directory: Option<PathBuf>,
    project: Vec<u8>,
    name: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    project: Vec<u8>,
    allowed: bool,
}

impl TrustStore {
    pub fn new(directory: Option<PathBuf>, project: &Path) -> io::Result<Self> {
        #[cfg(any(unix, windows))]
        let home = crate::user_paths::system_home_directory();
        #[cfg(not(any(unix, windows)))]
        let home: Option<PathBuf> = None;
        #[cfg(windows)]
        let standard = crate::user_paths::system_local_app_data_directory()
            .map(|path| path.join("runyte/cache/lsp-trust"));
        #[cfg(not(windows))]
        let standard = home.as_ref().map(|home| home.join(Self::HOME_CACHE));
        Self::new_with_account_paths(directory, project, home.as_deref(), standard.as_deref())
    }

    /// The account home is injected separately from environment-selected cache
    /// storage. An arbitrary XDG override inside a project is still rejected.
    #[cfg(all(test, unix))]
    pub(crate) fn new_with_home(
        directory: Option<PathBuf>,
        project: &Path,
        account_home: Option<&Path>,
    ) -> io::Result<Self> {
        let standard = account_home.map(|home| home.join(Self::HOME_CACHE));
        Self::new_with_account_paths(directory, project, account_home, standard.as_deref())
    }

    /// Account identity and its standard trust location are independent: the
    /// Windows local application-data folder may be redirected outside home.
    pub(crate) fn new_with_account_paths(
        directory: Option<PathBuf>,
        project: &Path,
        account_home: Option<&Path>,
        standard_trust_directory: Option<&Path>,
    ) -> io::Result<Self> {
        let project = project.canonicalize()?;
        if let Some(directory) = &directory {
            if !directory.is_absolute() {
                return Err(io::Error::other(
                    "LSP trust storage must have an absolute path",
                ));
            }
            #[cfg(windows)]
            let compared_directory = crate::path_safety::canonicalize_existing_prefix(directory)
                .map_err(io::Error::other)?;
            #[cfg(not(windows))]
            let compared_directory = directory.clone();
            #[cfg(windows)]
            let standard_trust_directory = standard_trust_directory
                .map(crate::path_safety::canonicalize_existing_prefix)
                .transpose()
                .map_err(io::Error::other)?;
            let home_cache = account_home.is_some_and(|home| {
                home.canonicalize().ok().as_ref() == Some(&project)
                    && standard_trust_directory
                        .as_ref()
                        .is_some_and(|standard| compared_directory == *standard)
            });
            if !home_cache {
                crate::project_root::validate_state_root(&project, &[compared_directory])
                    .map_err(io::Error::other)?;
            }
        }
        let project = project_bytes(&project);
        let name = format!("{}.json", crate::hash::sha256_hex(&project));
        Ok(Self {
            directory,
            project,
            name,
        })
    }

    #[cfg(target_os = "macos")]
    pub(crate) const HOME_CACHE: &str = "Library/Caches/runyte/lsp-trust";
    #[cfg(all(not(target_os = "macos"), not(windows)))]
    pub(crate) const HOME_CACHE: &str = ".cache/runyte/lsp-trust";

    pub(crate) fn can_remember(&self) -> bool {
        self.directory.is_some()
    }

    pub fn load(&self) -> io::Result<Option<bool>> {
        let Some(path) = &self.directory else {
            return Ok(None);
        };
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        let directory = crate::private_storage::Directory::open(path, true)?;
        let bytes = match directory.read(self.name.as_ref(), 64 * 1024) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let decision: Decision = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if decision.project != self.project {
            return Err(io::Error::other(
                "LSP trust record names a different workspace",
            ));
        }
        Ok(Some(decision.allowed))
    }

    /// Removes a saved decision so a one-time grant cannot preserve an older
    /// permanent approval. A missing store needs no write or directory creation.
    pub fn forget(&self) -> io::Result<()> {
        let Some(path) = &self.directory else {
            return Ok(());
        };
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        let directory = crate::private_storage::Directory::open(path, true)?;
        match directory.remove(self.name.as_ref()) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Ok(()) => directory.sync(),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, allowed: bool) -> io::Result<()> {
        let path = self
            .directory
            .as_ref()
            .ok_or_else(|| io::Error::other("private per-user LSP trust storage is unavailable"))?;
        let directory = crate::private_storage::Directory::open(path, true)?;
        let bytes = serde_json::to_vec(&Decision {
            project: self.project.clone(),
            allowed,
        })
        .map_err(io::Error::other)?;
        directory.atomic_write(self.name.as_ref(), &bytes)
    }
}

fn project_bytes(project: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        project.as_os_str().as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        project
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect()
    }
    #[cfg(not(any(unix, windows)))]
    {
        project.to_string_lossy().as_bytes().to_vec()
    }
}
