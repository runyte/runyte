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
