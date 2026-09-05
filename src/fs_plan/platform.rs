// SPDX-License-Identifier: MPL-2.0

//! Exclusive entry installation and copying through owned file handles.
//! This is not path confinement.

use std::{fs::File, io, path::Path};

/// Copy into an already exclusively created regular file. Never reopen its
/// pathname: publication and cleanup still own this exact destination handle.
#[cfg(target_os = "macos")]
pub(super) fn copy_file(source: &mut File, target: &mut File) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    // Match std::fs::copy's macOS metadata behavior, including resource forks,
    // extended attributes and ACLs. Setting permission bits after this call
    // could change the copied ACL, so native copying owns that step too.
    // SAFETY: both File values keep valid descriptors alive throughout the
    // call. A null state requests copyfile's internally managed default state.
    // No callback, pathname, or descriptor ownership is transferred.
    let result = unsafe {
        libc::fcopyfile(
            source.as_raw_fd(),
            target.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_DATA | libc::COPYFILE_METADATA,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        // Do not silently retry a metadata failure as a data-only copy.
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn copy_file(source: &mut File, target: &mut File) -> io::Result<()> {
    io::copy(source, target)?;
    target.set_permissions(source.metadata()?.permissions())
}

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
