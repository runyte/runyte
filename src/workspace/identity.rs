// SPDX-License-Identifier: MPL-2.0

use std::{fmt, fs, path::Path, path::PathBuf};

/// Length of the stable hexadecimal identity derived from a project root.
pub const WORKSPACE_ID_LENGTH: usize = 32;

/// Derives the stable identity for a canonical workspace project root.
///
/// One derivation serves the transport endpoint, the session catalog, and
/// diagnostic records, so a workspace is named the same way everywhere. The
/// path is hashed as operating-system bytes rather than as a display string:
/// two roots that differ only outside UTF-8 are still two workspaces. Unix
/// keeps raw bytes; Windows hashes the protocol's explicit UTF-16LE units.
pub fn workspace_id(project_root: &Path) -> String {
    #[cfg(any(unix, windows))]
    let bytes = crate::native_path::encode_path(project_root);
    #[cfg(not(any(unix, windows)))]
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
    #[cfg(unix)]
    fn unix_workspace_hash_keeps_original_raw_byte_vectors() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        assert_eq!(
            workspace_id(Path::new("/workspace/example")),
            "863c29bb9a6e7822955a80f7e09466e1"
        );
        let path = PathBuf::from(OsString::from_vec(b"/workspace/\xff".to_vec()));
        assert_eq!(workspace_id(&path), "25a377951b8ec475873878b73ff22d64");
    }

    #[test]
    #[cfg(windows)]
    fn windows_workspace_hash_preserves_distinct_unpaired_units_without_case_folding() {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        let first = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd800]));
        let second = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd801]));
        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        assert_ne!(workspace_id(&first), workspace_id(&second));
        let replacement = Path::new("C:\\\u{fffd}");
        assert_eq!(first.to_string_lossy(), replacement.to_string_lossy());
        assert_ne!(workspace_id(&first), workspace_id(replacement));
        assert_eq!(
            workspace_id(&first),
            crate::hash::sha256_hex(&[67, 0, 58, 0, 92, 0, 0, 216])[..WORKSPACE_ID_LENGTH]
        );
        assert_ne!(
            workspace_id(Path::new(r"C:\Example")),
            workspace_id(Path::new(r"C:\example"))
        );
    }

    #[test]
    fn equivalent_paths_have_one_workspace_identity() {
        let root = std::env::temp_dir();
        let direct = WorkspaceIdentity::resolve(&root).unwrap();
        let dotted = WorkspaceIdentity::resolve(root.join(".")).unwrap();
        assert_eq!(direct, dotted);
        assert_eq!(workspace_id(direct.root()), workspace_id(dotted.root()));
    }
}
