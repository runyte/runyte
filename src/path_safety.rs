// SPDX-License-Identifier: MPL-2.0

//! Canonical path checks shared by editor filesystem operations.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// Verifies that `candidate` remains inside `root` after resolving every
/// existing path component, including symlinks.
pub fn ensure_within_root(root: &Path, candidate: &Path) -> Result<()> {
    let canonical_root = canonicalize_existing_prefix(root)?;
    let canonical_candidate = canonicalize_existing_prefix(candidate)?;
    if canonical_candidate.starts_with(&canonical_root) {
        Ok(())
    } else {
        bail!(
            "{} resolves outside the project root {}",
            candidate.display(),
            root.display()
        )
    }
}

/// Canonicalises the longest existing prefix and re-appends components that
/// do not exist yet.
pub fn canonicalize_existing_prefix(path: &Path) -> Result<PathBuf> {
    let mut ancestor = path.to_path_buf();
    let mut trailing = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut canonical) => {
                for part in trailing.iter().rev() {
                    canonical.push(part);
                }
                return Ok(canonical);
            }
            Err(_) => match (ancestor.file_name(), ancestor.parent()) {
                (Some(name), Some(parent)) => {
                    trailing.push(name.to_os_string());
                    ancestor = parent.to_path_buf();
                }
                _ => return Ok(path.to_path_buf()),
            },
        }
    }
}

/// Resolves the filesystem identity used when deciding whether two editor
/// paths name one buffer.
///
/// An existing path is canonicalized, including symbolic links. A path that
/// cannot yet be resolved keeps its component sequence: in particular,
/// `missing/../file` is not equivalent to `file` until the missing directory
/// exists and the filesystem can actually traverse it.
pub fn path_identity(path: &Path) -> Result<PathBuf> {
    path_identity_with_depth(path, 0)
}

fn path_identity_with_depth(path: &Path, depth: usize) -> Result<PathBuf> {
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        anyhow::ensure!(depth < 40, "too many symbolic links in {}", path.display());
        let target = std::fs::read_link(path)?;
        let target = if target.is_absolute() {
            target
        } else {
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."))
                .join(target)
        };
        return path_identity_with_depth(&target, depth + 1);
    }
    canonicalize_existing_prefix(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_parent_components_are_not_lexically_cancelled() {
        let root = std::env::temp_dir().join(format!(
            "runyte-path-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();

        assert_ne!(
            path_identity(&root.join("missing/../note.txt")).unwrap(),
            path_identity(&root.join("note.txt")).unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_has_the_identity_of_its_missing_target() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "runyte-dangling-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("target.txt");
        let link = root.join("link.txt");
        symlink("target.txt", &link).unwrap();

        assert_eq!(
            path_identity(&link).unwrap(),
            path_identity(&target).unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_file_under_a_symlinked_parent_uses_the_real_parent_identity() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "runyte-symlinked-parent-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let real = root.join("real");
        let alias = root.join("alias");
        std::fs::create_dir_all(&real).unwrap();
        symlink("real", &alias).unwrap();

        assert_eq!(
            path_identity(&alias.join("new.txt")).unwrap(),
            path_identity(&real.join("new.txt")).unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
