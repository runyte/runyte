// SPDX-License-Identifier: MPL-2.0

//! Git's ordinary path spelling and the editor's canonical filesystem identity.

use std::{
    io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use super::{GitError, Result};

fn error(path: &Path, source: io::Error) -> GitError {
    GitError::Io {
        action: "resolve a native Git directory at",
        path: path.to_path_buf(),
        detail: source.to_string(),
    }
}

pub(super) fn identity(path: &Path) -> Result<PathBuf> {
    path.canonicalize().map_err(|source| error(path, source))
}

/// Resolve the existing prefix, retaining missing worktree entries for review.
pub(super) fn worktree_identity(path: &Path, missing: bool) -> Result<PathBuf> {
    let resolve = || {
        let (ancestor, suffix) = existing_prefix(path).map_err(|source| error(path, source))?;
        Ok(identity(ancestor)?.join(suffix))
    };
    match resolve() {
        // An offline drive/share must not hide other worktrees or branches.
        // Keep Git's unresolved spelling for inspection; mutation still needs
        // a resolvable ordinary directory operand and a fresh guarded review.
        Err(_) if missing => Ok(path.to_path_buf()),
        result => result,
    }
}

fn existing_prefix(path: &Path) -> io::Result<(&Path, PathBuf)> {
    let mut ancestor = path;
    let mut names = Vec::new();
    loop {
        match ancestor.try_exists() {
            Ok(true) => return Ok((ancestor, names.into_iter().rev().collect())),
            Ok(false) => {
                let name = ancestor.file_name().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Git directory has no existing ancestor",
                    )
                })?;
                names.push(name);
                ancestor = ancestor.parent().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Git directory has no existing ancestor",
                    )
                })?;
            }
            Err(source) => return Err(source),
        }
    }
}

/// Convert only directory operands. Revision names and literal pathspecs never
/// pass through this function. Validate before a worktree creates a branch.
pub(super) fn argument(base: &Path, path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let convert = || -> io::Result<PathBuf> {
        let (ancestor, suffix) = existing_prefix(&absolute)?;
        crate::windows_fs::validate_relative(&suffix)?;
        let ordinary = crate::windows_fs::ordinary_working_directory(ancestor)?.join(suffix);
        if ordinary.as_os_str().encode_wide().count() >= 260 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Git directory requires an ordinary Windows path shorter than 260 UTF-16 units",
            ));
        }
        Ok(ordinary)
    };
    convert().map_err(|source| error(path, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestRuntimeRoot;

    #[test]
    fn existing_and_missing_worktrees_keep_canonical_unicode_identity() {
        let fixture = TestRuntimeRoot::new("git-paths").unwrap();
        let path = fixture.path().join("checkout λ");
        std::fs::create_dir(&path).unwrap();
        let ordinary = argument(fixture.path(), &path).unwrap();
        assert_eq!(
            worktree_identity(&ordinary, false).unwrap(),
            path.canonicalize().unwrap()
        );
        let missing = ordinary.join("missing").join("child");
        assert_eq!(
            worktree_identity(&missing, true).unwrap(),
            path.canonicalize().unwrap().join("missing").join("child")
        );
    }

    #[test]
    fn an_unavailable_root_remains_listed_but_cannot_be_a_mutation_operand() {
        // A nonexistent volume gives a deterministic offline-root case without
        // contacting a network share or depending on assigned drive letters.
        let path = Path::new(r"\\?\Volume{00000000-0000-0000-0000-000000000000}\checkout");
        assert_eq!(worktree_identity(path, true).unwrap(), path);
        assert!(worktree_identity(path, false).is_err());
        assert!(argument(path, path).is_err());
    }

    #[test]
    fn directory_operands_refuse_names_with_no_ordinary_windows_equivalent() {
        let fixture = TestRuntimeRoot::new("git-operands").unwrap();
        for name in ["trailing.", "trailing ", "NUL", "stream:name", "wild*card"] {
            assert!(
                argument(fixture.path(), &fixture.path().join(name)).is_err(),
                "{name}"
            );
        }
        assert!(argument(fixture.path(), &fixture.path().join("x".repeat(250))).is_err());
        assert!(argument(fixture.path(), Path::new("nested/checkout λ")).is_ok());
    }
}
