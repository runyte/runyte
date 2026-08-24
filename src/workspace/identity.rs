// SPDX-License-Identifier: MPL-2.0

use std::{fmt, fs, path::Path, path::PathBuf};

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
