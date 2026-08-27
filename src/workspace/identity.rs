// SPDX-License-Identifier: MPL-2.0

use std::{fmt, fs, path::Path, path::PathBuf};

/// Length of the stable hexadecimal identity derived from a project root.
pub const WORKSPACE_ID_LENGTH: usize = 32;

/// Derives the stable identity for a canonical workspace project root.
///
/// One derivation serves the transport endpoint, the session catalog, and
/// diagnostic records, so a workspace is named the same way everywhere. The
/// path is hashed as operating-system bytes rather than as a display string:
/// two roots that differ only outside UTF-8 are still two workspaces.
pub fn workspace_id(project_root: &Path) -> String {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        project_root.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = project_root.to_string_lossy().into_owned().into_bytes();
    crate::hash::sha256_hex(&bytes)[..WORKSPACE_ID_LENGTH].to_owned()
}

/// Canonical project-root identity for one workspace host.
///
/// The path remains an operating-system value rather than a lossy display
/// string. Endpoint hashing belongs to the transport phase; this core identity
/// can therefore preserve non-UTF-8 roots on platforms that support them.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceIdentity(PathBuf);

impl WorkspaceIdentity {
    pub fn resolve(root: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self(fs::canonicalize(root)?))
    }

    pub fn from_canonical(root: PathBuf) -> Self {
        Self(root)
    }

    pub fn root(&self) -> &Path {
        &self.0
    }
}

impl fmt::Display for WorkspaceIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_paths_have_one_workspace_identity() {
        let root = std::env::temp_dir();
        let direct = WorkspaceIdentity::resolve(&root).unwrap();
        let dotted = WorkspaceIdentity::resolve(root.join(".")).unwrap();
        assert_eq!(direct, dotted);
    }
}
