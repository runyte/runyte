// SPDX-License-Identifier: MPL-2.0

//! Operating-system account paths used for owner-scoped state.

use std::{ffi::CStr, os::unix::ffi::OsStringExt, path::PathBuf, sync::OnceLock};

/// The effective operating-system account's home directory.
///
/// `$HOME` may belong to the invoking account rather than the process's
/// effective user, most notably when a command is run through `sudo`. State
/// owned by the effective user must therefore resolve the account database
/// directly instead of trusting inherited environment.
pub(crate) fn system_home_directory() -> Option<PathBuf> {
    static HOME: OnceLock<PathBuf> = OnceLock::new();

    if let Some(home) = HOME.get() {
        return Some(home.clone());
    }
    let resolved = resolve_system_home_directory()?;
    // Another caller may have won the race with the same effective account.
    // Retain that answer when it did; otherwise retain this successful one.
    let _ = HOME.set(resolved.clone());
    HOME.get().cloned().or(Some(resolved))
}

fn resolve_system_home_directory() -> Option<PathBuf> {
    // SAFETY: `sysconf` reads one process configuration value and has no
    // pointer preconditions.
    let configured = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let mut capacity = if configured > 0 {
        usize::try_from(configured).ok()?
    } else {
        16 * 1024
    }
    .clamp(1024, 1024 * 1024);
    loop {
        let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let mut storage = vec![0_u8; capacity];
        // SAFETY: `record`, `storage`, and `result` are live writable storage;
        // the buffer length matches the allocation and the UID is valid.
        let status = unsafe {
            libc::getpwuid_r(
                libc::geteuid(),
                record.as_mut_ptr(),
                storage.as_mut_ptr().cast::<libc::c_char>(),
                storage.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE && capacity < 1024 * 1024 {
            capacity = (capacity * 2).min(1024 * 1024);
            continue;
        }
        if status != 0 || result.is_null() {
            return None;
        }
        // SAFETY: a successful `getpwuid_r` initialized `record` and returned
        // its address through `result`.
        if unsafe { (*result).pw_dir.is_null() } {
            return None;
        }
        // SAFETY: the successful lookup placed a NUL-terminated directory
        // string inside `storage`, which remains alive for this copy.
        let directory = unsafe { CStr::from_ptr((*result).pw_dir) };
        let path = PathBuf::from(std::ffi::OsString::from_vec(directory.to_bytes().to_vec()));
        return (path.is_absolute() && !path.as_os_str().is_empty()).then_some(path);
    }
}
