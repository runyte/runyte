// SPDX-License-Identifier: MPL-2.0

//! Existing Unix history storage; the shared codec owns content validation.

use super::MAX_RECENTS_BYTES;
use anyhow::{Context, Result};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read as _},
    path::{Path, PathBuf},
};

pub(super) struct LockedHistory {
    path: PathBuf,
    _lock: RecentFileLock,
}

impl LockedHistory {
    pub(super) fn acquire(path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_owned(),
            _lock: RecentFileLock::acquire(path)?,
        })
    }
    pub(super) fn read(&self) -> Result<Vec<u8>> {
        match read(&self.path) {
            Ok(bytes) => Ok(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(b"[]".to_vec()),
            Err(error) => Err(error.into()),
        }
    }
    pub(super) fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let path = &self.path;
        let parent = path
            .parent()
            .context("workspace recents path has no parent")?;
        prepare_parent(parent)?;
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&temporary, bytes)?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
        fs::rename(temporary, path)?;
        Ok(())
    }
}

/// A dedicated advisory lock for the recents file, not for the cache or user
/// directory around it. The kernel releases `flock` when this descriptor is
/// closed, including process exit after a crash; the persistent lock file is
/// inert and can safely be reused by the next process.
pub(in crate::workspace) struct RecentFileLock(fs::File);

impl RecentFileLock {
    pub(in crate::workspace) fn acquire(recents: &Path) -> Result<Self> {
        Self::acquire_with_operation(recents, libc::LOCK_EX)?.ok_or_else(|| {
            anyhow::anyhow!("blocking workspace recents lock unexpectedly was unavailable")
        })
    }

    fn acquire_with_operation(recents: &Path, operation: libc::c_int) -> Result<Option<Self>> {
        use std::os::{
            fd::AsRawFd,
            unix::fs::{OpenOptionsExt, PermissionsExt},
        };

        let Some(parent) = recents.parent() else {
            anyhow::bail!("workspace recents path has no parent")
        };
        prepare_parent(parent)?;
        let path = recents.with_extension("lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .with_context(|| format!("cannot open workspace recents lock {}", path.display()))?;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .with_context(|| format!("cannot secure workspace recents lock {}", path.display()))?;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
                return Ok(Some(Self(file)));
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if operation & libc::LOCK_NB != 0 && error.kind() == io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(error)
                .with_context(|| format!("cannot lock workspace recents file {}", path.display()));
        }
    }

    #[cfg(test)]
    pub(in crate::workspace) fn try_acquire(recents: &Path) -> Result<Option<Self>> {
        Self::acquire_with_operation(recents, libc::LOCK_EX | libc::LOCK_NB)
    }
}

impl Drop for RecentFileLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;

        let _ = unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

pub(super) fn prepare_parent(parent: &Path) -> Result<()> {
    fs::create_dir_all(parent)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub(super) fn read(path: &Path) -> io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_RECENTS_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RECENTS_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("workspace recents exceed {MAX_RECENTS_BYTES} bytes"),
        ));
    }
    Ok(bytes)
}
