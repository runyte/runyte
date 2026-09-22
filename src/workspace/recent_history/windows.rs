// SPDX-License-Identifier: MPL-2.0

//! Private, regenerable native history. A transaction retains its admitted
//! directory and stable lock until the bounded read/modify/replace is complete.

use super::MAX_RECENTS_BYTES;
use crate::{private_storage::Directory, windows_fs::Identity};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read},
    os::windows::io::AsRawHandle,
    path::Path,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::ERROR_LOCK_VIOLATION,
    Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx, UnlockFileEx,
    },
    System::IO::OVERLAPPED,
};

const LOCK_BUDGET: Duration = Duration::from_secs(2);
const LOCK_RETRY: Duration = Duration::from_millis(10);

pub(super) fn prepare_parent(parent: &Path) -> Result<()> {
    Directory::open(parent, true)?;
    Ok(())
}

fn leaf(path: &Path) -> io::Result<&OsStr> {
    path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "workspace recents path has no filename",
        )
    })
}

pub(super) fn read(path: &Path) -> io::Result<Vec<u8>> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "workspace recents path has no parent",
        )
    })?;
    let directory = Directory::open_existing(parent, true)?;
    read_at(&directory, leaf(path)?)
}

fn read_at(directory: &Directory, name: &OsStr) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    directory
        .open_read(name)?
        .take((MAX_RECENTS_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RECENTS_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("workspace recents exceed {MAX_RECENTS_BYTES} bytes"),
        ));
    }
    Ok(bytes)
}

pub(super) struct LockedHistory {
    directory: Directory,
    name: OsString,
    lock_name: OsString,
    lock: FileLock,
}

impl LockedHistory {
    pub(super) fn acquire(path: &Path) -> Result<Self> {
        Self::acquire_with_budget(path, LOCK_BUDGET)
    }

    fn acquire_with_budget(path: &Path, budget: Duration) -> Result<Self> {
        let parent = path
            .parent()
            .context("workspace recents path has no parent")?;
        let directory = Directory::open(parent, true)?;
        let name = leaf(path)?.to_owned();
        let lock_name = leaf(&path.with_extension("lock"))?.to_owned();
        // Reject a caller-supplied name which would alias its own lock file.
        ensure!(
            !name
                .to_string_lossy()
                .eq_ignore_ascii_case(&lock_name.to_string_lossy()),
            "workspace recents path aliases its lock"
        );
        let file = directory.append(&lock_name)?;
        let deadline = Instant::now() + budget;
        let lock = loop {
            match FileLock::try_lock(&file)? {
                true => break FileLock(file),
                false if Instant::now() >= deadline => {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "workspace recents lock is busy",
                    )
                    .into());
                }
                false => std::thread::sleep(
                    LOCK_RETRY.min(deadline.saturating_duration_since(Instant::now())),
                ),
            }
        };
        let value = Self {
            directory,
            name,
            lock_name,
            lock,
        };
        value.verify_lock()?;
        Ok(value)
    }

    fn verify_lock(&self) -> Result<()> {
        let current = self.directory.open_read(&self.lock_name)?;
        ensure!(
            Identity::of(&current)? == Identity::of(&self.lock.0)?,
            "workspace recents lock was replaced"
        );
        Ok(())
    }

    pub(super) fn read(&self) -> Result<Vec<u8>> {
        self.verify_lock()?;
        match read_at(&self.directory, &self.name) {
            Ok(bytes) => Ok(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(b"[]".to_vec()),
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.verify_lock()?;
        ensure!(
            bytes.len() <= MAX_RECENTS_BYTES,
            "workspace recents exceed {MAX_RECENTS_BYTES} bytes"
        );
        self.directory.atomic_write(&self.name, bytes)?;
        Ok(())
    }
}

struct FileLock(File);

impl FileLock {
    fn try_lock(file: &File) -> io::Result<bool> {
        let mut offset = lock_offset();
        if unsafe {
            LockFileEx(
                file.as_raw_handle(),
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut offset,
            )
        } != 0
        {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
            Ok(false)
        } else {
            Err(error)
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // The stable file is never removed or replaced by a history operation.
        unsafe {
            UnlockFileEx(self.0.as_raw_handle(), 0, 1, 0, &mut lock_offset());
        }
    }
}

fn lock_offset() -> OVERLAPPED {
    let mut offset: OVERLAPPED = unsafe { std::mem::zeroed() };
    offset.Anonymous.Anonymous.Offset = u32::MAX - 1;
    offset.Anonymous.Anonymous.OffsetHigh = u32::MAX;
    offset
}

#[cfg(test)]
#[path = "windows/tests.rs"]
mod tests;
