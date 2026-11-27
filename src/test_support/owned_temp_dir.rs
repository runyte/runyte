// SPDX-License-Identifier: MPL-2.0

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// A unique test-owned child of the system temporary directory. Cleanup never
/// accepts a caller-supplied path, and rechecks containment before removal.
#[derive(Debug)]
pub struct OwnedTempDir {
    base: PathBuf,
    path: PathBuf,
    component: OsString,
}

impl OwnedTempDir {
    pub fn new() -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = fs::canonicalize(std::env::temp_dir())?;
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..128 {
            let component = OsString::from(format!(
                "runyte-owned-{}-{time:x}-{:x}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let path = base.join(&component);
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        base,
                        path,
                        component,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate unique test directory",
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn validate_cleanup(&self) -> io::Result<()> {
        let owned = self.path.parent() == Some(self.base.as_path())
            && self.path.file_name() == Some(self.component.as_os_str())
            && self
                .component
                .to_string_lossy()
                .starts_with("runyte-owned-")
            && fs::symlink_metadata(&self.path)?.file_type().is_dir()
            && fs::canonicalize(&self.path)? == self.path;
        if !owned {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing cleanup outside the allocated test directory",
            ));
        }
        Ok(())
    }

    pub fn cleanup(&self) -> io::Result<()> {
        self.validate_cleanup()?;
        fs::remove_dir_all(&self.path)
    }
}

impl Drop for OwnedTempDir {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_accepts_only_the_unique_owned_child() {
        let mut root = OwnedTempDir::new().unwrap();
        let owned = root.path.clone();
        fs::write(owned.join("sentinel"), "owned").unwrap();
        let other = OwnedTempDir::new().unwrap();
        for rejected in [
            root.base.clone(),
            root.base.join(".."),
            other.path.clone(),
            owned.join("child"),
        ] {
            root.path = rejected;
            assert!(root.cleanup().is_err());
            assert!(owned.join("sentinel").exists());
            assert!(other.path.exists());
        }
        root.path = owned.clone();
        root.cleanup().unwrap();
        assert!(!owned.exists());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_refuses_a_replaced_symlink() {
        let root = OwnedTempDir::new().unwrap();
        let other = OwnedTempDir::new().unwrap();
        fs::write(other.path.join("sentinel"), "unrelated").unwrap();
        fs::remove_dir(&root.path).unwrap();
        std::os::unix::fs::symlink(&other.path, &root.path).unwrap();
        assert!(root.cleanup().is_err());
        assert!(other.path.join("sentinel").exists());
        fs::remove_file(&root.path).unwrap();
    }
}
