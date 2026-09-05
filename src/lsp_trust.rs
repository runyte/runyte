// SPDX-License-Identifier: MPL-2.0

//! Per-user approval to run language servers in one exact workspace.
//! The project cannot grant itself approval through a file in its own tree.

use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

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
        let project = project.canonicalize()?;
        if let Some(directory) = &directory {
            if !directory.is_absolute() {
                return Err(io::Error::other(
                    "LSP trust storage must have an absolute path",
                ));
            }
            crate::project_root::validate_state_root(&project, std::slice::from_ref(directory))
                .map_err(io::Error::other)?;
        }
        #[cfg(unix)]
        let project = {
            use std::os::unix::ffi::OsStrExt;
            project.as_os_str().as_bytes().to_vec()
        };
        #[cfg(not(unix))]
        let project = project.to_string_lossy().as_bytes().to_vec();
        let name = format!("{}.json", crate::hash::sha256_hex(&project));
        Ok(Self {
            directory,
            project,
            name,
        })
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
