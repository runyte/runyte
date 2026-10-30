// SPDX-License-Identifier: MPL-2.0

use crate::private_storage::Directory;
use std::{ffi::OsStr, fs::File, io};

pub(super) fn acquire(directory: &Directory) -> anyhow::Result<File> {
    let file = directory.append(OsStr::new(".agent-channels.initialize.lock"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
    loop {
        match lock(&file) {
            Ok(()) => return Ok(file),
            Err(error) if contended(&error) => {
                if std::time::Instant::now() >= deadline {
                    return Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                        None,
                    )
                    .into());
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn contended(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.kind() == io::ErrorKind::WouldBlock
    }
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(33)
    }
}

pub(super) fn ensure_file(directory: &Directory, name: &str) -> anyhow::Result<()> {
    match directory.create_new(OsStr::new(name)) {
        Ok(file) => drop(file),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let file = directory.open_read(OsStr::new(name))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                anyhow::ensure!(
                    file.metadata()?.permissions().mode() & 0o077 == 0,
                    "channel storage must be owner-private"
                );
            }
            drop(file);
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub(super) fn sidecars(directory: &Directory) -> anyhow::Result<()> {
    ensure_file(directory, "agent-channels.sqlite3-wal")?;
    ensure_file(directory, "agent-channels.sqlite3-shm")?;
    directory.sync()?;
    Ok(())
}

pub(super) fn persist_wal(db: &rusqlite::Connection) -> anyhow::Result<()> {
    let mut enabled: std::ffi::c_int = 1;
    // SAFETY: the connection and main schema name are live; SQLite reads/writes
    // one integer synchronously and does not retain the pointer.
    let result = unsafe {
        rusqlite::ffi::sqlite3_file_control(
            db.handle(),
            c"main".as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_PERSIST_WAL,
            (&mut enabled as *mut std::ffi::c_int).cast(),
        )
    };
    anyhow::ensure!(
        result == rusqlite::ffi::SQLITE_OK,
        "cannot retain private SQLite WAL: {result}"
    );
    Ok(())
}

#[cfg(unix)]
fn lock(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: the descriptor is live and flock takes no pointers.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn lock(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx},
        System::IO::OVERLAPPED,
    };
    // SAFETY: zero initializes the unused event and byte offset; the file handle
    // and OVERLAPPED pointer remain live throughout this synchronous call.
    let mut offset: OVERLAPPED = unsafe { std::mem::zeroed() };
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
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
