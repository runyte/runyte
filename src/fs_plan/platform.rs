// SPDX-License-Identifier: MPL-2.0

//! Exclusive installation of a directory entry. This is not path confinement.

use std::{io, path::Path};

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn rename_noreplace(source: &Path, target: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    // SAFETY: both paths are NUL-terminated and remain alive for the call.
    // AT_FDCWD is a valid directory selector; no borrowed descriptors escape.
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    // SAFETY: the same path and descriptor preconditions apply on macOS.
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        // Preserve errno, including EEXIST, EXDEV and unsupported flags. There
        // is deliberately no fallback to an overwriting rename.
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn rename_noreplace(_source: &Path, _target: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "exclusive filesystem-plan rename is unsupported on this platform",
    ))
}
