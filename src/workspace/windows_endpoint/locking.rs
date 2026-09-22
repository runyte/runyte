// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::{
    Foundation::ERROR_LOCK_VIOLATION,
    Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx, LOCKFILE_EXCLUSIVE_LOCK,
        LOCKFILE_FAIL_IMMEDIATELY, LockFileEx, UnlockFileEx,
    },
    System::IO::OVERLAPPED,
};

pub(super) type FileKey = (u64, [u8; 16]);

pub(super) fn file_key(file: &File) -> io::Result<FileKey> {
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok((info.VolumeSerialNumber, info.FileId.Identifier))
}

#[derive(Debug)]
pub(super) struct Lock(File);

impl Lock {
    pub(super) fn acquire(file: File) -> io::Result<Self> {
        let mut offset = offset();
        if unsafe {
            LockFileEx(
                file.as_raw_handle(),
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut offset,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            return Err(
                if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
                    io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "workspace publication lock is busy",
                    )
                } else {
                    error
                },
            );
        }
        Ok(Self(file))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // Explicit release avoids depending on delayed process-exit cleanup.
        // The lock file itself is stable and must never be deleted or replaced.
        let mut offset = offset();
        unsafe {
            UnlockFileEx(self.0.as_raw_handle(), 0, 1, 0, &mut offset);
        }
    }
}

fn offset() -> OVERLAPPED {
    let mut value: OVERLAPPED = unsafe { std::mem::zeroed() };
    value.Anonymous.Anonymous.Offset = u32::MAX - 1;
    value.Anonymous.Anonymous.OffsetHigh = u32::MAX;
    value
}

pub(super) fn acquire(
    roots: &[RegistryRoot],
    name: impl Fn(&RegistryRoot) -> String,
) -> io::Result<Vec<Lock>> {
    let mut files = roots
        .iter()
        .map(|root| {
            let file = root.directory.append(OsStr::new(&name(root)))?;
            Ok((file_key(&file)?, file))
        })
        .collect::<io::Result<Vec<_>>>()?;
    // Native identity orders aliases alike; a common secondary registry is
    // locked even when each contender has a different primary registry.
    files.sort_by_key(|(key, _)| *key);
    files.dedup_by_key(|(key, _)| *key);
    files
        .into_iter()
        .map(|(_, file)| Lock::acquire(file))
        .collect()
}
